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

    /// Deterministic precedence between copies of one record that
    /// differ only in state (#4578): the lifecycle advances
    /// `Recorded` -> `Submitted`, and a terminal decision outranks an
    /// in-flight state — `Posted` (a ledger fact) above `Abandoned`
    /// (a decision not to resubmit), so a decision recorded without
    /// knowledge of a completed posting never masks it, and an
    /// abandon decision is never undone by an older in-flight copy.
    pub fn precedence(self) -> u8 {
        match self {
            IntentState::Recorded => 0,
            IntentState::Submitted => 1,
            IntentState::Abandoned => 2,
            IntentState::Posted => 3,
        }
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
/// Two refinements from the Codex re-review of PR #4622: a path that
/// is a filesystem ALIAS of the canonical file (another spelling of
/// the same file on a case-insensitive filesystem, a symlink, a
/// hardlink) is the canonical file, never a stray copy to clean up;
/// and a conversion whose canonical file currently stores ANOTHER
/// conversion's record is blocked — `unresolved` skips and reports
/// it like a conflicting copy instead of handing recovery a record
/// whose state-advancing `put` is guaranteed to fail.
/// The next re-review round added: the scan's extension match is
/// case-insensitive (an uppercase-extension alias is scanned, never
/// invisible), the unreadable-canonical check in `get` matches
/// filesystem aliases and casing variants of the canonical name,
/// and occupant resolution goes through a per-scan index instead
/// of pairwise `same_file` calls (linear, not quadratic, I/O).
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
            // The extension match is case-insensitive (Codex P1 on
            // PR #4622): on a case-insensitive filesystem a file
            // named with an uppercase .JSON extension can BE a
            // canonical intent file, and excluding it from the scan
            // would hide its conversion's record from every guard
            // this contract backs.
            let is_json = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
            if !is_json {
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
        // Occupant index (Codex P2 on PR #4622): resolve every
        // entry's filesystem identity ONCE here - its own path, its
        // canonicalized path, and (on Unix) its device+inode - so
        // entry_at is a map lookup per query. The previous shape
        // called same_file() (canonicalize + metadata syscalls) for
        // every (entry, query) pair, which made unresolved()
        // quadratic in the number of stored records even when the
        // listing came back empty.
        let mut by_path: HashMap<PathBuf, usize> = HashMap::new();
        let mut by_file_id: HashMap<(u64, u64), usize> = HashMap::new();
        for (index, entry) in scanned.iter().enumerate() {
            by_path.entry(entry.path.clone()).or_insert(index);
            if let Ok(canon) = fs::canonicalize(&entry.path) {
                by_path.entry(canon).or_insert(index);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(meta) = fs::metadata(&entry.path) {
                    by_file_id.entry((meta.dev(), meta.ino())).or_insert(index);
                }
            }
        }
        Ok(IntentScan {
            entries: scanned,
            unreadable,
            by_path,
            by_file_id,
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
    /// Occupant index: path (as scanned, or canonicalized) -> entry.
    by_path: HashMap<PathBuf, usize>,
    /// Occupant index: (device, inode) -> entry; populated on Unix,
    /// empty elsewhere. Covers hardlinks, whose canonical paths
    /// still differ in spelling.
    by_file_id: HashMap<(u64, u64), usize>,
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

    /// The scanned entry sitting at `path`, whatever conversion its
    /// content names (#4578): a conversion's canonical file can hold
    /// ANOTHER conversion's record (a misnamed write or an operator
    /// restore), and that occupancy is a fact about the path, not
    /// about the content identity `copies_for` groups by.
    fn entry_at(&self, path: &Path) -> Option<&ScannedIntent> {
        if let Some(&index) = self.by_path.get(path) {
            return Some(&self.entries[index]);
        }
        if let Ok(canon) = fs::canonicalize(path) {
            if let Some(&index) = self.by_path.get(&canon) {
                return Some(&self.entries[index]);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = fs::metadata(path) {
                if let Some(&index) = self.by_file_id.get(&(meta.dev(), meta.ino())) {
                    return Some(&self.entries[index]);
                }
            }
        }
        // Lexical alias: a file whose NAME differs from the query
        // path's name only in casing spells the same canonical name
        // (the hex stem is case-insensitive, like the extension),
        // so it is the canonical occupant even where the filesystem
        // keeps the spellings distinct. String compares only - no
        // syscalls - so this never reintroduces the quadratic I/O
        // the index above removed.
        let name = path.file_name()?;
        self.entries.iter().find(|entry| {
            entry.path.file_name().is_some_and(|other| {
                other
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            })
        })
    }

    /// The unreadable entry sitting at `path`, with the same alias
    /// matching `entry_at` applies (Codex P2 on PR #4622): an
    /// unreadable canonical file spelled with different casing -
    /// uppercase hex digits, say - is still this conversion's
    /// canonical file, so `get` must fail closed on it instead of
    /// treating the conversion as absent.
    fn unreadable_at(&self, path: &Path) -> Option<&UnreadableIntent> {
        self.unreadable.iter().find(|entry| {
            if entry.path == path || same_file(&entry.path, path) {
                return true;
            }
            match (entry.path.file_name(), path.file_name()) {
                (Some(a), Some(b)) => a
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&b.to_string_lossy()),
                _ => false,
            }
        })
    }

    /// The one distinct record among `copies`, or `None` when the
    /// copies disagree — an ambiguous identity (#4578).
    ///
    /// Copies that differ ONLY in state are one record at its most
    /// advanced state (by [`IntentState::precedence`]): a
    /// state-advancing `put` writes the new canonical record before
    /// removing superseded copies, so a crash — or a failed removal —
    /// in between leaves exactly that residue, and the conversion
    /// must stay readable and reconcilable instead of failing closed
    /// forever (Codex P2 on PR #4622). Copies that differ in any
    /// other field are genuinely conflicting records: ambiguous.
    fn distinct(copies: &[&ScannedIntent]) -> Option<IntentRecord> {
        let first = &copies.first()?.record;
        let mut resolved = first.clone();
        for copy in copies {
            let mut comparable = copy.record.clone();
            comparable.state = first.state;
            if comparable != *first {
                return None;
            }
            if copy.record.state.precedence() > resolved.state.precedence() {
                resolved.state = copy.record.state;
            }
        }
        Some(resolved)
    }
}

/// True when two paths name the same filesystem object (Codex P1
/// on PR #4622): on a case-insensitive filesystem the canonical
/// file's uppercase-hex spelling is the SAME file, and a symlink or
/// hardlink to it resolves to it too. Lexical path comparison
/// misses all of these, and treating such an alias as a stray copy
/// is destructive: after `put` renames the new record over the
/// canonical path, removing the "stray" alias would delete the
/// record just persisted. Canonicalisation covers spellings the OS
/// itself resolves to one path; device+inode covers hardlinks and
/// symlinks whose canonical paths still differ in spelling.
fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(canon_a), Ok(canon_b)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        if canon_a == canon_b {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(meta_a), Ok(meta_b)) = (fs::metadata(a), fs::metadata(b)) {
            return meta_a.dev() == meta_b.dev() && meta_a.ino() == meta_b.ino();
        }
    }
    false
}

/// The fail-closed error for a canonical file that currently stores
/// a DIFFERENT conversion's record (#4578, Codex P1 on PR #4622):
/// treating the conversion as absent would let `prepare()` pass its
/// never-replace guard, and overwriting would destroy the occupant's
/// only write-ahead record.
fn canonical_occupant_error(
    conversion_id: u128,
    canonical: &Path,
    occupant_id: u128,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "intent store: canonical intent file {} for conversion {conversion_id} \
             currently stores conversion {occupant_id}'s record; refusing to treat \
             conversion {conversion_id} as absent or overwrite the occupant — \
             reconcile the file manually (its content names the conversion it \
             belongs to)",
            canonical.display()
        ),
    )
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
        // Canonical occupancy (#4578): a record belonging to another
        // conversion sitting at this conversion's canonical path is
        // never overwritten — it may be that conversion's only
        // write-ahead record.
        if let Some(occupant) = scan.entry_at(&canonical) {
            if occupant.record.conversion_id != record.conversion_id {
                return Err(canonical_occupant_error(
                    record.conversion_id,
                    &canonical,
                    occupant.record.conversion_id,
                ));
            }
        }
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
            // Alias guard (Codex P1 on PR #4622): a "stray" that is
            // the canonical file under another spelling or link is
            // not a superseded copy — since the rename above, it
            // names the record just persisted, so removing it would
            // delete the write-ahead record `put` just made durable.
            // Leave it in place; listings group by content identity,
            // so the alias never double-counts the conversion.
            if same_file(stray, &canonical) {
                continue;
            }
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
        if let Some(bad) = scan.unreadable_at(&canonical) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "intent store: canonical intent file {} is unreadable: {}",
                    canonical.display(),
                    bad.reason
                ),
            ));
        }
        // Canonical occupancy (#4578): if this conversion's
        // canonical file holds another conversion's record, the
        // conversion is NOT absent — answering None here would let
        // prepare()'s never-replace guard pass and put() overwrite
        // the occupant's only record. Fail closed instead.
        if let Some(occupant) = scan.entry_at(&canonical) {
            if occupant.record.conversion_id != conversion_id {
                return Err(canonical_occupant_error(
                    conversion_id,
                    &canonical,
                    occupant.record.conversion_id,
                ));
            }
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
            // Canonical occupancy (Codex P2 on PR #4622): when this
            // conversion's canonical file currently stores ANOTHER
            // conversion's record, `get` and `put` for this
            // conversion fail closed by design — so a listing that
            // still yields the record hands recovery an entry whose
            // state-advancing `put` is guaranteed to error out and
            // abort the whole scan. Skip the blocked conversion and
            // report it through `unreadable_entries()` (the record's
            // own copies stay on disk for an operator), exactly like
            // a conflicting copy below; every other conversion still
            // reconciles.
            let canonical = self.path_for(conversion_id);
            if let Some(occupant) = scan.entry_at(&canonical) {
                if occupant.record.conversion_id != conversion_id {
                    skipped.push(UnreadableIntent {
                        path: canonical,
                        reason: format!(
                            "conversion {conversion_id} is blocked: its canonical                              intent file currently stores conversion {}'s record,                              so the conversion was skipped during recovery listing;                              reconcile the file manually (its content names the                              conversion it belongs to)",
                            occupant.record.conversion_id
                        ),
                    });
                    continue;
                }
            }
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
