//! The ingest contract, shared by the property tests and the regression replay: the
//! same events, in any order and with any repeats, give a store the same applied state.
//! The checks take bytes, so a case found by a property run can be replayed unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use id::chain::decode;
use id::event_store::{Chain, EventStore, Storage};
use id::waiting::AuthorKey;
use p2panda_core::{Hash, SeqNum};
use redb::backends::InMemoryBackend;
use redb::{Database, DatabaseError, StorageBackend, StorageError};

/// The applied state of a set of authors, as a store reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Canonical {
    /// Each author's applied chain. An author with no applied event and no fork has no row.
    pub chains: BTreeMap<AuthorKey, Chain>,
    /// Each author's fork proof: the fork point, and the two IDs in order.
    pub proofs: BTreeMap<AuthorKey, (SeqNum, Hash, Hash)>,
    /// The IDs of every applied event, across all authors.
    pub applied: BTreeSet<Hash>,
}

/// How long one delivery may take before it counts as a hang. A store that spins on an
/// event fails the check, instead of hanging the whole test run.
pub const DELIVERY_LIMIT: Duration = Duration::from_secs(20);

/// Reads the applied state of `authors` from `store`.
///
/// # Errors
///
/// Returns the store's error as text, if a read fails.
pub fn canonical(store: &EventStore, authors: &[AuthorKey]) -> Result<Canonical, String> {
    let mut state = Canonical::default();
    for author in authors {
        if let Some(chain) = store.chain(author).map_err(|err| err.to_string())? {
            for seq in 0..chain.next_seq {
                let event = store.event(author, seq).map_err(|err| err.to_string())?;
                let event = event.ok_or_else(|| format!("applied sequence {seq} is not stored"))?;
                state.applied.insert(event.id);
            }
            state.chains.insert(*author, chain);
        }
        if let Some(proof) = store.fork_proof(author).map_err(|err| err.to_string())? {
            state
                .proofs
                .insert(*author, (proof.seq, proof.first.id, proof.second.id));
        }
    }
    Ok(state)
}

/// The authors of the events in `order`, sorted. Bytes that do not decode have no
/// author, and are skipped.
pub fn authors_of(order: &[Vec<u8>]) -> Vec<AuthorKey> {
    let mut authors: Vec<AuthorKey> = order
        .iter()
        .filter_map(|bytes| decode(bytes).ok())
        .map(|event| *event.header.verifying_key.as_bytes())
        .collect();
    authors.sort_unstable();
    authors.dedup();
    authors
}

/// A store directory in memory. Each file is a redb in-memory backend, so no write is
/// synced to a disk. The store's creation path runs unchanged, through the same
/// [`Storage`] calls the disk makes.
#[derive(Debug, Default)]
pub struct MemStorage {
    files: Mutex<BTreeMap<String, Arc<InMemoryBackend>>>,
}

/// One in-memory file, shared by the databases that open it.
#[derive(Debug)]
struct Shared(Arc<InMemoryBackend>);

impl StorageBackend for Shared {
    fn len(&self) -> Result<u64, io::Error> {
        self.0.len()
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        self.0.read(offset, out)
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        self.0.set_len(len)
    }

    fn sync_data(&self) -> Result<(), io::Error> {
        self.0.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        self.0.write(offset, data)
    }
}

impl MemStorage {
    fn files(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<InMemoryBackend>>> {
        self.files.lock().expect("memory directory lock")
    }
}

impl Storage for MemStorage {
    fn create_database(&self, name: &str) -> Result<Database, DatabaseError> {
        let file = Arc::clone(
            self.files()
                .entry(name.to_owned())
                .or_insert_with(|| Arc::new(InMemoryBackend::new())),
        );
        Database::builder().create_with_backend(Shared(file))
    }

    fn open_database(&self, name: &str) -> Result<Database, DatabaseError> {
        let file = self.files().get(name).cloned().ok_or_else(|| {
            DatabaseError::Storage(StorageError::Io(io::Error::from(io::ErrorKind::NotFound)))
        })?;
        Database::builder().create_with_backend(Shared(file))
    }

    fn exists(&self, name: &str) -> io::Result<bool> {
        Ok(self.files().contains_key(name))
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        self.files().remove(name);
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let mut files = self.files();
        let file = files
            .remove(from)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        files.insert(to.to_owned(), file);
        Ok(())
    }

    fn sync_file(&self, _name: &str) -> io::Result<()> {
        Ok(())
    }

    fn sync_dir(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Delivers `order` to a fresh in-memory store on a fixed clock, and returns the applied
/// state of `authors`. The store's invariants must hold after the last delivery. The
/// authors are passed in, not decoded from `order`, because every decode verifies a
/// signature, and a debug build spends most of its time there.
///
/// # Errors
///
/// Describes the first delivery or invariant that fails.
pub fn deliver(order: &[Vec<u8>], authors: &[AuthorKey]) -> Result<Canonical, String> {
    let mut store = EventStore::open_with(&MemStorage::default()).map_err(|err| err.to_string())?;
    for bytes in order {
        store
            .ingest(bytes, 0)
            .map_err(|err| format!("ingest failed: {err}"))?;
    }
    store
        .check_invariants()
        .map_err(|err| format!("invariants after the deliveries: {err}"))?;
    canonical(&store, authors)
}

/// [`deliver`] on a helper thread. Fails if the delivery has not returned within `limit`.
///
/// # Errors
///
/// Returns the delivery's error, or a message when it did not finish in time or panicked.
pub fn deliver_within(
    order: Vec<Vec<u8>>,
    authors: Vec<AuthorKey>,
    limit: Duration,
) -> Result<Canonical, String> {
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        // The receiver is gone only once the wait below has given up.
        let _ = sent.send(deliver(&order, &authors));
    });
    match received.recv_timeout(limit) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(format!(
            "the delivery did not finish within {limit:?}: an event made the store spin"
        )),
        Err(RecvTimeoutError::Disconnected) => Err("the delivery panicked".to_owned()),
    }
}

/// Checks that delivery order and repeats do not change the applied state. Each
/// event is delivered twice in a row, once in the given order, once reversed, and
/// once rotated by half the length.
///
/// # Errors
///
/// Describes the first order that disagrees with the others, or the first failure.
pub fn check_order_independence(events: &[Vec<u8>]) -> Result<(), String> {
    let twice = |order: &[Vec<u8>]| -> Vec<Vec<u8>> {
        order
            .iter()
            .flat_map(|bytes| [bytes.clone(), bytes.clone()])
            .collect()
    };
    let forward = events.to_vec();
    let mut reversed = events.to_vec();
    reversed.reverse();
    let mut rotated = events.to_vec();
    if !rotated.is_empty() {
        let half = rotated.len() / 2;
        rotated.rotate_left(half);
    }
    let authors = authors_of(events);
    let first = deliver_within(twice(&forward), authors.clone(), DELIVERY_LIMIT)?;
    for (name, order) in [("reversed", &reversed), ("rotated", &rotated)] {
        let other = deliver_within(twice(order), authors.clone(), DELIVERY_LIMIT)?;
        if other != first {
            return Err(format!(
                "the {name} order reaches {other:?}, but the given order reaches {first:?}"
            ));
        }
    }
    Ok(())
}
