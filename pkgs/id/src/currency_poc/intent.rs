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

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

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

/// Persistence seam for intent records.
pub trait IntentStore {
    /// Insert or replace a record. Must be durable before returning:
    /// the write-ahead guarantee depends on it.
    fn put(&mut self, record: &IntentRecord) -> io::Result<()>;
    fn get(&self, conversion_id: u128) -> io::Result<Option<IntentRecord>>;
    fn unresolved(&self) -> io::Result<Vec<IntentRecord>>;
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
}

impl FileIntentStore {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
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
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let record: IntentRecord = serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if record.state.is_unresolved() {
                records.push(record);
            }
        }
        Ok(records)
    }
}
