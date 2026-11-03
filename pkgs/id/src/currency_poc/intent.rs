//! Write-ahead intent records for the conversion-executor PoC.
//!
//! The executor persists an [`IntentRecord`] — conversion ID, the
//! pre-generated transfer IDs of every leg, amounts, and the
//! authorisation ID — *before* any ledger submission, and advances its
//! state as submission proceeds. A crash therefore always leaves a
//! durable record the recovery scanner (`super::recovery`) can
//! reconcile against the ledger by transfer-ID lookup: present means
//! POSTED, absent means the same IDs may be safely resubmitted
//! (TigerBeetle transfer creation is idempotent by ID) or the record
//! abandoned.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ledger::{AccountId, Transfer};

/// Lifecycle of a conversion intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentState {
    /// Persisted, not yet submitted. A crash in this state means the
    /// ledger was never touched; resubmission uses the same IDs.
    Recorded,
    /// Submission was attempted; the outcome is unknown until the
    /// scanner (or a retry) looks the transfer IDs up.
    Submitted,
    /// Every leg is present in the ledger.
    Posted,
    /// The scanner (or an operator) decided not to resubmit.
    Abandoned,
}

impl IntentState {
    /// States the recovery scanner still has to reconcile.
    pub fn is_unresolved(self) -> bool {
        matches!(self, IntentState::Recorded | IntentState::Submitted)
    }
}

/// The write-ahead record for one conversion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentRecord {
    pub conversion_id: u128,
    /// The full legs, with their pre-generated transfer IDs. Storing
    /// the legs (not just the IDs) is what lets the scanner resubmit
    /// byte-identical transfers after a crash.
    pub legs: Vec<Transfer>,
    pub user_source: AccountId,
    pub lp_source: AccountId,
    pub fee_account: Option<AccountId>,
    pub lp_dest: AccountId,
    pub user_dest: AccountId,
    pub source_ledger: u32,
    pub dest_ledger: u32,
    pub amount_in: u128,
    pub amount_out: u128,
    pub fee_amount: u128,
    /// The control-plane rate authorisation this conversion executes
    /// under (#2063: rates and authorisations live outside TigerBeetle).
    pub authorisation_id: String,
    pub state: IntentState,
}

impl IntentRecord {
    pub fn transfer_ids(&self) -> Vec<u128> {
        self.legs.iter().map(|leg| leg.id).collect()
    }
}

/// A stored entry that a listing skipped because it could not be
/// read or parsed (#4125). Skipped entries are never deleted by the
/// store or the recovery scanner: the path stays on disk so an
/// operator can inspect, repair, or remove it deliberately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableIntent {
    pub path: PathBuf,
    pub reason: String,
}

/// Persistence seam for intent records.
pub trait IntentStore {
    /// Insert or replace a record. Must be durable before returning:
    /// the write-ahead guarantee depends on it.
    fn put(&mut self, record: &IntentRecord) -> io::Result<()>;
    fn get(&self, conversion_id: u128) -> io::Result<Option<IntentRecord>>;
    /// List every unresolved record in the store.
    ///
    /// Listing contract (#4125): one unreadable or unparseable entry
    /// must NOT abort the listing \u2014 the store directory is a
    /// long-lived recovery surface that has to survive schema
    /// evolution, operator handling, and foreign files. An
    /// implementation therefore skips such an entry, leaves it in
    /// place, and reports it through
    /// [`IntentStore::unreadable_entries`]; only a failure to
    /// enumerate the store itself is an `Err`. The in-memory store
    /// holds already-parsed records, so it never skips.
    fn unresolved(&self) -> io::Result<Vec<IntentRecord>>;
    /// Entries skipped by the most recent [`IntentStore::unresolved`]
    /// call because they could not be read or parsed, in path order.
    /// Empty when the last listing skipped nothing (or for stores
    /// that cannot hold unreadable entries).
    fn unreadable_entries(&self) -> Vec<UnreadableIntent> {
        Vec::new()
    }
}

/// In-memory store for tests and demos.
#[derive(Default)]
pub struct InMemoryIntentStore {
    records: HashMap<u128, IntentRecord>,
}

impl InMemoryIntentStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IntentStore for InMemoryIntentStore {
    fn put(&mut self, record: &IntentRecord) -> io::Result<()> {
        self.records.insert(record.conversion_id, record.clone());
        Ok(())
    }

    fn get(&self, conversion_id: u128) -> io::Result<Option<IntentRecord>> {
        Ok(self.records.get(&conversion_id).cloned())
    }

    fn unresolved(&self) -> io::Result<Vec<IntentRecord>> {
        Ok(self
            .records
            .values()
            .filter(|record| record.state.is_unresolved())
            .cloned()
            .collect())
    }
}

/// File-backed store: one JSON file per conversion under a directory,
/// written via a temporary file + rename + fsync so a crash mid-write
/// never leaves a torn record behind.
pub struct FileIntentStore {
    dir: PathBuf,
    /// Entries skipped by the most recent `unresolved` listing
    /// (#4125); behind a `RefCell` because listing takes `&self`.
    unreadable: RefCell<Vec<UnreadableIntent>>,
}

impl FileIntentStore {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            unreadable: RefCell::new(Vec::new()),
        })
    }

    fn note_unreadable(&self, path: &Path, reason: String) {
        self.unreadable.borrow_mut().push(UnreadableIntent {
            path: path.to_path_buf(),
            reason,
        });
    }

    fn path_for(&self, conversion_id: u128) -> PathBuf {
        self.dir.join(format!("{conversion_id:032x}.json"))
    }
}

impl IntentStore for FileIntentStore {
    fn put(&mut self, record: &IntentRecord) -> io::Result<()> {
        let path = self.path_for(record.conversion_id);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&tmp, bytes)?;
        let file = fs::File::open(&tmp)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &path)?;
        // Sync the containing directory so the rename itself is
        // durable: syncing the file alone does not guarantee the new
        // directory entry survives a machine crash, and the
        // write-ahead guarantee depends on the record being findable
        // by the recovery scanner afterwards.
        let dir = fs::File::open(&self.dir)?;
        dir.sync_all()?;
        Ok(())
    }

    fn get(&self, conversion_id: u128) -> io::Result<Option<IntentRecord>> {
        let path = self.path_for(conversion_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let record = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Some(record))
    }

    fn unresolved(&self) -> io::Result<Vec<IntentRecord>> {
        // A failure to enumerate the directory itself is fatal (the
        // store is unreachable); anything wrong with one entry inside
        // it is not (#4125).
        let entries = fs::read_dir(&self.dir)?;
        self.unreadable.borrow_mut().clear();
        let mut records = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.note_unreadable(&self.dir, format!("directory entry: {error}"));
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            // fs::read also fails for a subdirectory named *.json;
            // serde parsing fails for foreign or schema-drifted
            // content. Both skip-and-report, never abort, never delete.
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.note_unreadable(&path, format!("read: {error}"));
                    continue;
                }
            };
            let record: IntentRecord = match serde_json::from_slice(&bytes) {
                Ok(record) => record,
                Err(error) => {
                    self.note_unreadable(&path, format!("parse: {error}"));
                    continue;
                }
            };
            if record.state.is_unresolved() {
                records.push(record);
            }
        }
        self.unreadable
            .borrow_mut()
            .sort_by(|a, b| a.path.cmp(&b.path));
        Ok(records)
    }

    fn unreadable_entries(&self) -> Vec<UnreadableIntent> {
        self.unreadable.borrow().clone()
    }
}
