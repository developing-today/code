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
use std::collections::{BTreeMap, HashMap};
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
/// read or parsed (#4125), or because its identity conflicts with
/// another stored copy of the same conversion (#4578). Skipped
/// entries are never deleted by the store or the recovery scanner:
/// the path stays on disk so an operator can inspect, repair, or
/// remove it deliberately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableIntent {
    pub path: PathBuf,
    pub reason: String,
}

/// Persistence seam for intent records.
///
/// Identity contract (#4578): a `conversion_id` names ONE conversion,
/// and every implementation must agree on that identity across
/// `put`, `get`, and `unresolved`. Duplicate stored copies of one
/// record are one record (a listing never yields it twice), and two
/// different records stored under one conversion_id are an ambiguous
/// identity that must fail closed — never silently shadow, bypass,
/// or double-process each other.
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
///
/// Filesystem identity contract (#4578): a conversion's identity is
/// the `conversion_id` in the record CONTENT, and its canonical file
/// is `{conversion_id:032x}.json` — the only path this store ever
/// writes. Every read scans the directory and groups by content ID,
/// so `get`, `unresolved`, `put`, and the executor's `prepare` guard
/// (which is `get`-based) all see the same identity surface:
/// duplicate copies of one record are one record; a record stored
/// only under a non-canonical name is still the conversion's record;
/// different records under one conversion_id fail closed with a
/// diagnostic naming the paths (see the method implementations).
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

    fn path_for(&self, conversion_id: u128) -> PathBuf {
        self.dir.join(format!("{conversion_id:032x}.json"))
    }

    /// Scan the whole store directory once: every `*.json` file that
    /// parses, in path order, plus every entry that could not be read
    /// or parsed (#4125). Only a failure to enumerate the directory
    /// itself is an `Err` — the store is unreachable then; anything
    /// wrong with one entry inside it is reported, never fatal here.
    fn scan(&self) -> io::Result<IntentScan> {
        let entries = fs::read_dir(&self.dir)?;
        let mut paths = Vec::new();
        let mut unreadable = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(error) => unreadable.push(UnreadableIntent {
                    path: self.dir.clone(),
                    reason: format!("directory entry: {error}"),
                }),
            }
        }
        paths.sort();
        let mut scanned = Vec::new();
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            // fs::read also fails for a subdirectory named *.json;
            // serde parsing fails for foreign or schema-drifted
            // content. Both skip-and-report, never abort, never delete.
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    unreadable.push(UnreadableIntent {
                        path,
                        reason: format!("read: {error}"),
                    });
                    continue;
                }
            };
            match serde_json::from_slice::<IntentRecord>(&bytes) {
                Ok(record) => scanned.push(ScannedIntent { path, record }),
                Err(error) => unreadable.push(UnreadableIntent {
                    path,
                    reason: format!("parse: {error}"),
                }),
            }
        }
        unreadable.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(IntentScan {
            entries: scanned,
            unreadable,
        })
    }
}

/// One parseable intent file found by a directory scan.
struct ScannedIntent {
    path: PathBuf,
    record: IntentRecord,
}

/// The result of one [`FileIntentStore::scan`].
struct IntentScan {
    entries: Vec<ScannedIntent>,
    unreadable: Vec<UnreadableIntent>,
}

impl IntentScan {
    /// Every stored copy of `conversion_id`, in path order. Identity
    /// is the conversion_id in the record CONTENT (#4578): the file
    /// name a copy sits under does not change which conversion it is.
    fn copies_for(&self, conversion_id: u128) -> Vec<&ScannedIntent> {
        self.entries
            .iter()
            .filter(|entry| entry.record.conversion_id == conversion_id)
            .collect()
    }

    /// The one distinct record among `copies`, or `None` when the
    /// copies disagree — an ambiguous identity (#4578).
    fn distinct(copies: &[&ScannedIntent]) -> Option<IntentRecord> {
        let first = &copies.first()?.record;
        copies
            .iter()
            .all(|copy| copy.record == *first)
            .then(|| first.clone())
    }
}

/// The fail-closed error for an ambiguous conversion identity: two
/// different records stored under one conversion_id (#4578).
fn ambiguous_identity_error(conversion_id: u128, copies: &[&ScannedIntent]) -> io::Error {
    let paths: Vec<String> = copies
        .iter()
        .map(|copy| copy.path.display().to_string())
        .collect();
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "intent store: conversion {conversion_id} has conflicting records at {};              one conversion_id must name one record — reconcile the copies manually              (keep the record that matches the ledger, remove the others)",
            paths.join(", ")
        ),
    )
}

impl IntentStore for FileIntentStore {
    fn put(&mut self, record: &IntentRecord) -> io::Result<()> {
        let canonical = self.path_for(record.conversion_id);
        // Identity check (#4578): before writing, every stored copy
        // of this conversion — canonical or not — must be the same
        // conversion this write belongs to. State is the one field a
        // legitimate write may advance, so it is normalised out of
        // the comparison (the same trick prepare() uses): a copy
        // that differs in anything else is a conflicting record, and
        // a conflicting put fails closed and writes nothing.
        let scan = self.scan()?;
        let copies = scan.copies_for(record.conversion_id);
        let mut stray_paths: Vec<PathBuf> = Vec::new();
        if !copies.is_empty() {
            let Some(stored) = IntentScan::distinct(&copies) else {
                return Err(ambiguous_identity_error(record.conversion_id, &copies));
            };
            let mut comparable = stored;
            comparable.state = record.state;
            if comparable != *record {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "intent store: refusing to write conversion {} over a different                          persisted intent (stored at {}); a conversion_id names one                          conversion — reconcile the stored record first",
                        record.conversion_id,
                        copies[0].path.display()
                    ),
                ));
            }
            stray_paths = copies
                .iter()
                .map(|copy| copy.path.clone())
                .filter(|path| *path != canonical)
                .collect();
        }
        let tmp = canonical.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&tmp, bytes)?;
        let file = fs::File::open(&tmp)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &canonical)?;
        // Sync the containing directory so the rename itself is
        // durable: syncing the file alone does not guarantee the new
        // directory entry survives a machine crash, and the
        // write-ahead guarantee depends on the record being findable
        // by the recovery scanner afterwards.
        let dir = fs::File::open(&self.dir)?;
        dir.sync_all()?;
        // Canonicalize (#4578): the write above supersedes every
        // stray copy of this same record, so remove them — one
        // conversion now lives at exactly its canonical path, and no
        // later listing can meet the same identity twice. Copies of
        // a DIFFERENT record never reach this point (refused above),
        // and unreadable entries are never deleted (#4125).
        for stray in &stray_paths {
            fs::remove_file(stray).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "intent store: conversion {} is persisted at {}, but removing                          its superseded duplicate copy {} failed: {error}; remove it                          manually so the conversion has exactly one stored copy",
                        record.conversion_id,
                        canonical.display(),
                        stray.display()
                    ),
                )
            })?;
        }
        if !stray_paths.is_empty() {
            let dir = fs::File::open(&self.dir)?;
            dir.sync_all()?;
        }
        Ok(())
    }

    fn get(&self, conversion_id: u128) -> io::Result<Option<IntentRecord>> {
        let scan = self.scan()?;
        // A corrupt canonical file fails closed, as it always has:
        // it is the record this conversion's write-ahead guarantee
        // rests on, so treating it as absent could let prepare()
        // overwrite an intent that may already be posted. (A corrupt
        // NON-canonical file is a #4125 skipped entry, not this
        // conversion's canonical record.)
        let canonical = self.path_for(conversion_id);
        if let Some(bad) = scan.unreadable.iter().find(|entry| entry.path == canonical) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "intent store: canonical intent file {} is unreadable: {}",
                    canonical.display(),
                    bad.reason
                ),
            ));
        }
        let copies = scan.copies_for(conversion_id);
        if copies.is_empty() {
            return Ok(None);
        }
        // Duplicate copies of one record are that record; a record
        // stored only under a non-canonical name is still found, so
        // prepare()'s never-replace guard cannot be bypassed by a
        // misnamed file (#4578). Copies that disagree are an
        // ambiguous identity and fail closed.
        match IntentScan::distinct(&copies) {
            Some(record) => Ok(Some(record)),
            None => Err(ambiguous_identity_error(conversion_id, &copies)),
        }
    }

    fn unresolved(&self) -> io::Result<Vec<IntentRecord>> {
        let scan = self.scan()?;
        let mut skipped = scan.unreadable.clone();
        // Group by the conversion_id the content names (#4578):
        // duplicate copies collapse to one record, so the recovery
        // scanner can never reconcile one conversion twice. A
        // conversion whose copies disagree is ambiguous — listing
        // any one of them would risk reconciling the wrong record —
        // so it is skipped and every copy is reported through
        // unreadable_entries() while the rest of the listing
        // proceeds (#4125 listing contract).
        let mut by_conversion: BTreeMap<u128, Vec<&ScannedIntent>> = BTreeMap::new();
        for entry in &scan.entries {
            by_conversion
                .entry(entry.record.conversion_id)
                .or_default()
                .push(entry);
        }
        let mut records = Vec::new();
        for (conversion_id, copies) in by_conversion {
            match IntentScan::distinct(&copies) {
                Some(record) => {
                    if record.state.is_unresolved() {
                        records.push(record);
                    }
                }
                None => {
                    for copy in copies {
                        skipped.push(UnreadableIntent {
                            path: copy.path.clone(),
                            reason: format!(
                                "conflicting records for conversion {conversion_id}:                                  more than one distinct record is stored under this                                  conversion_id, so its identity is ambiguous and it                                  was skipped; reconcile the copies manually"
                            ),
                        });
                    }
                }
            }
        }
        skipped.sort_by(|a, b| a.path.cmp(&b.path));
        *self.unreadable.borrow_mut() = skipped;
        Ok(records)
    }

    fn unreadable_entries(&self) -> Vec<UnreadableIntent> {
        self.unreadable.borrow().clone()
    }
}
