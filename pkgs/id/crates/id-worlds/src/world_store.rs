//! Durable worlds: an append-only journal plus content-addressed modules.
//!
//! A world directory holds:
//!
//! ```text
//! <dir>/journal.jsonl            one JournalEntry per line, synced per the
//!                                journal's ack mode (fsynced per append
//!                                in the default mode)
//! <dir>/modules/<blake3>.wasm    every module the world has installed
//! ```
//!
//! A world is restored by rebuilding the core from the journal,
//! instantiating the last installed module with its recorded seed, and
//! replaying the inputs committed after that install. That is exact because
//! world programs are pure: the same module, seed and inputs always produce
//! the same state.
//!
//! Replaying from the beginning gets slower forever, so a program that can
//! serialize its own state (`plaza_snapshot` / `plaza_restore`) lets the world
//! trim its journal: the whole history is replaced, atomically, by a
//! checkpoint holding that state, and restore resumes from it. The host
//! verifies each snapshot round-trips on a probe instance before trimming.
//!
//! The journal records capability digests, never the bearer secrets.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::world::{
    JOURNAL_VERSION, JournalAckMode, JournalEntry, WorldCore, WorldEventKind, WorldHandle,
    WorldJournal, WorldLimits, WorldProgram, journal_version_required_by,
};
use crate::world_session::{WorldService, module_hash};

/// Production byte cap for one world's journal file: the F12.18
/// journal-size bound (#560, option B, decided 2026-11-16).
///
/// Checkpoint trimming (`WorldLimits::checkpoint_every`) keeps a healthy
/// journal far below this — a checkpoint replaces the whole history with
/// one snapshot entry — so the cap is a backstop, not a sizing tool: it
/// binds only when trimming cannot run (a program with no snapshot
/// support, a world that never checkpoints). When an append — or a
/// journal transaction, refused whole — would cross the cap it is
/// refused before anything is written; the remedy is the world's own
/// checkpoint/trim or transaction rewrite, which re-baselines the
/// tracked size through `compact`, and writes resume once the journal
/// is back under the cap.
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

/// Appends per sync in [`JournalAckMode::Batched`]: the journal issues
/// one `sync_data` per this many appends (counted across `append` and
/// `append_batch` entries), so at most `JOURNAL_ACK_BATCH_SIZE - 1`
/// acknowledged appends are ever waiting on their batch's sync. The
/// bound is the mode's whole loss window — an OS crash can drop the
/// unsynced tail, never more than one partial batch — so it is a
/// durability choice, not a tuning knob: 64 inputs is ~3 s of one
/// world's F12.12 input rate (20 ev/s), small enough to lose, large
/// enough to cut syncs per input by ~64x where sync latency binds.
pub const JOURNAL_ACK_BATCH_SIZE: u64 = 64;

/// Append-only JSON-lines journal, synced per its [`JournalAckMode`]
/// (fsynced per append in the default mode).
#[derive(Debug)]
pub struct FileJournal {
    file: File,
    path: PathBuf,
    /// Set when a compaction left durability uncertain; appends then fail so
    /// no acknowledged event can be lost.
    broken: bool,
    /// The version in the journal's `Created` header, kept in step with the
    /// bump-on-first-write contract (see `world::JOURNAL_VERSION`).
    version: u32,
    /// Bytes currently in the journal file, tracked across appends and
    /// compactions so the cap check never stats the file on the hot path.
    bytes: u64,
    /// The byte cap this journal enforces (see [`MAX_JOURNAL_BYTES`]).
    max_bytes: u64,
    /// When appends are acknowledged relative to the sync that makes
    /// them durable, chosen at open (see [`JournalAckMode`]). Never
    /// changes over the journal's life and never affects how existing
    /// bytes are read back.
    ack_mode: JournalAckMode,
    /// Appends written since the last completed sync. Always zero in
    /// [`JournalAckMode::Sync`]; in the relaxed modes it is exactly
    /// the set of acknowledged appends an OS crash could still lose,
    /// and it returns to zero at every sync point (a batch-boundary
    /// sync, [`WorldJournal::sync`], a transaction, a compaction).
    unsynced_appends: u64,
    /// Lock on a stable sibling file. The journal itself is replaced by
    /// compaction, so locking its inode would not protect the path lifetime.
    /// Keep this last so the journal file is closed before ownership is
    /// released.
    _lock: File,
}

/// Run `op` under the worker-parking guard the first #557 fix
/// established for journal I/O: on a multi-thread runtime the
/// blocking I/O runs under `block_in_place`, so the runtime hands
/// this worker's other tasks to a replacement thread while the caller
/// waits; everywhere else (plain threads, `spawn_blocking`,
/// current-thread test runtimes) it is direct. Without this, W
/// runtime workers cap the whole hub at W appends per sync latency —
/// the #557 many-worlds profile measured exactly that ceiling
/// (4 workers / 46 ms ≈ 87 appends/s against 2,000 offered).
fn in_durability_guard<T>(op: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(op)
    } else {
        op()
    }
}

/// Write `line` to the journal file and sync it durable — the whole
/// durable cost of one append (one `write_all` + one `sync_data`).
///
/// The write+sync runs inline in the caller, under the
/// [`in_durability_guard`] worker-parking guard.
///
/// The journal used to hand each append to a dedicated writer thread
/// over a channel and rendezvous on its reply. That hop bought
/// nothing: the journal's only producer is its owning actor, which
/// reaches it through `&mut` and cannot have a second append in flight
/// while one waits for durability, so the writer's queue was empty at
/// every dequeue by construction and each append paid a channel send,
/// a thread wake, and a rendezvous reply around the same sync. Writing
/// inline removes those hops (and one thread per world) with no
/// durability or ordering change: an append is still acknowledged
/// only after the sync that covers its bytes completes.
fn sync_journal_write(file: &mut File, line: &[u8]) -> std::io::Result<()> {
    in_durability_guard(|| file.write_all(line).and_then(|()| file.sync_data()))
}

/// Write `line` to the journal file WITHOUT syncing it — the relaxed
/// [`JournalAckMode`]s' half of an append. The bytes are in the OS
/// (and survive a process crash) but are not durable until a later
/// sync covers them.
fn write_journal_line(file: &mut File, line: &[u8]) -> std::io::Result<()> {
    in_durability_guard(|| file.write_all(line))
}

/// Sync the journal file, making every byte written so far durable —
/// the deferred half of a relaxed-mode append, run at the mode's sync
/// points.
fn sync_journal_file(file: &mut File) -> std::io::Result<()> {
    in_durability_guard(|| file.sync_data())
}

fn acquire_journal_lock(path: &Path) -> Result<File> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let lock = options
        .open(&lock_path)
        .with_context(|| format!("open world journal lock {}", lock_path.display()))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!(
                "world journal {} is already open by another handle (lock {})",
                path.display(),
                lock_path.display()
            )
        }
        Err(std::fs::TryLockError::Error(error)) => Err(error).with_context(|| {
            format!(
                "lock world journal {} via {}",
                path.display(),
                lock_path.display()
            )
        }),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalPartition {
    version: u32,
    world_id: String,
    jurisdiction: String,
}

// Caller holds the stable journal lock. Never infer a binding for a legacy
// journal; the sidecar is committed before the first journal bytes exist.
fn check_journal_partition(path: &Path, world_id: &str, jurisdiction: Option<&str>) -> Result<()> {
    let mut metadata_path = path.as_os_str().to_owned();
    metadata_path.push(".partition.json");
    let metadata_path = PathBuf::from(metadata_path);
    match std::fs::symlink_metadata(&metadata_path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file(),
                "world partition unavailable"
            );
            let stored: JournalPartition = serde_json::from_slice(&std::fs::read(&metadata_path)?)
                .map_err(|_| anyhow::anyhow!("world partition unavailable"))?;
            ensure!(
                stored.version == 1
                    && stored.world_id == world_id
                    && jurisdiction == Some(stored.jurisdiction.as_str()),
                "world partition unavailable"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(jurisdiction) = jurisdiction {
                ensure!(!path.try_exists()?, "world partition unavailable");
                let record = JournalPartition {
                    version: 1,
                    world_id: world_id.to_owned(),
                    jurisdiction: jurisdiction.to_owned(),
                };
                let parent = metadata_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let mut file = tempfile::NamedTempFile::new_in(parent)?;
                file.write_all(&serde_json::to_vec(&record)?)?;
                file.as_file().sync_all()?;
                file.persist_noclobber(&metadata_path)?;
                #[cfg(unix)]
                File::open(parent)?.sync_all()?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Whether the durable world at `dir` is bound to `jurisdiction`
/// (#3898). This is a read-only peek for discovery (existence checks
/// and listings), so a partition-scoped server can treat a world bound
/// elsewhere, an unassigned legacy world, a malformed binding and a
/// missing world exactly alike: as not there. It takes no lock and
/// grants nothing; [`FileJournal::open_in_partition`] still re-checks
/// the binding under the journal lock before any byte is read.
#[must_use]
pub fn world_bound_to_partition(dir: &Path, world_id: &str, jurisdiction: &str) -> bool {
    let journal = dir.join("journal.jsonl");
    let sidecar = dir.join("journal.jsonl.partition.json");
    let is_plain_file = |path: &Path| {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
    };
    if !is_plain_file(&journal) || !is_plain_file(&sidecar) {
        return false;
    }
    std::fs::read(&sidecar)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JournalPartition>(&bytes).ok())
        .is_some_and(|stored| {
            stored.version == 1
                && stored.world_id == world_id
                && stored.jurisdiction == jurisdiction
        })
}

impl FileJournal {
    /// Open (or create) a journal and return its entries.
    ///
    /// A torn final line, left by a crash mid-append, is truncated away: that
    /// entry was never acknowledged. Corruption anywhere else is an error, so
    /// history is never silently dropped.
    ///
    /// # Errors
    ///
    /// I/O failures, or a malformed entry before the last line.
    pub fn open(path: &Path, world_id: &str) -> Result<(Self, Vec<JournalEntry>)> {
        Self::open_impl(
            path,
            world_id,
            MAX_JOURNAL_BYTES,
            JournalAckMode::Sync,
            None,
        )
    }

    /// Open (or create) a journal enforcing an explicit byte cap.
    /// Production callers use [`FileJournal::open`] and the production
    /// cap ([`MAX_JOURNAL_BYTES`]); the explicit form exists for tests
    /// and operator tooling. A journal already over the cap still opens
    /// — its history is readable and a checkpoint can trim it — but
    /// writes that would grow it refuse until it is back under the cap.
    pub fn open_with_max_bytes(
        path: &Path,
        world_id: &str,
        max_bytes: u64,
    ) -> Result<(Self, Vec<JournalEntry>)> {
        Self::open_impl(path, world_id, max_bytes, JournalAckMode::Sync, None)
    }

    /// Open (or create) a journal running the given acknowledgement
    /// mode (see [`JournalAckMode`]) under the production byte cap.
    /// The mode governs only what future appends promise; the entries
    /// already in the journal are read back exactly as any other mode
    /// would read them.
    pub fn open_with_ack_mode(
        path: &Path,
        world_id: &str,
        ack_mode: JournalAckMode,
    ) -> Result<(Self, Vec<JournalEntry>)> {
        Self::open_impl(path, world_id, MAX_JOURNAL_BYTES, ack_mode, None)
    }

    /// Open a journal with an immutable, explicitly supplied jurisdiction.
    /// The caller must resolve this value from trusted account/partition
    /// configuration, not a client-supplied header. This is a storage fence,
    /// not account admission or proof that a partition is provisioned.
    ///
    /// # Errors
    ///
    /// Refuses invalid labels, unassigned existing journals, different stored
    /// world/partition bindings, malformed metadata, or I/O failures. Refusal
    /// happens before journal reads, tail repair or append access.
    pub fn open_in_partition(
        path: &Path,
        world_id: &str,
        jurisdiction: &str,
        ack_mode: JournalAckMode,
    ) -> Result<(Self, Vec<JournalEntry>)> {
        // Same label syntax as hosted_migration; provisioning is caller-owned.
        ensure!(
            !jurisdiction.is_empty()
                && jurisdiction.len() <= 63
                && jurisdiction
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "world partition unavailable"
        );
        Self::open_impl(
            path,
            world_id,
            MAX_JOURNAL_BYTES,
            ack_mode,
            Some(jurisdiction),
        )
    }

    fn open_impl(
        path: &Path,
        world_id: &str,
        max_bytes: u64,
        ack_mode: JournalAckMode,
        jurisdiction: Option<&str>,
    ) -> Result<(Self, Vec<JournalEntry>)> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let lock = acquire_journal_lock(path)?;
        // All opens reject aliases: an unscoped symlink would otherwise look
        // beside the alias rather than beside the assigned target journal.
        ensure!(
            !std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
            "world partition unavailable"
        );
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(path) {
            use std::os::unix::fs::MetadataExt;
            ensure!(metadata.nlink() == 1, "world partition unavailable");
        }
        check_journal_partition(path, world_id, jurisdiction)?;
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("open world journal {}", path.display()))?;

        let (mut entries, good_len, torn) = {
            let mut reader = BufReader::new(&file);
            Self::scan_entries(&mut reader, path)?
        };
        if torn {
            tracing::warn!(
                "world journal {}: dropping torn final entry at byte {good_len}",
                path.display()
            );
            file.set_len(good_len)?;
            file.sync_data()?;
        }
        file.seek(SeekFrom::End(0))?;

        let version = match entries.first() {
            Some(JournalEntry::Created { version, .. }) => *version,
            _ => JOURNAL_VERSION,
        };
        let mut journal = Self {
            file,
            path: path.to_owned(),
            broken: false,
            version,
            bytes: good_len,
            max_bytes,
            ack_mode,
            unsynced_appends: 0,
            _lock: lock,
        };
        if entries.is_empty() {
            let header = JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            };
            journal.append(&header)?;
            entries.push(header);
        }
        // Journals written before the issue-#227 contract can already hold
        // an entry a version-1 binary cannot read under a version-1 header.
        // Bump such a journal on open, so a later rollback to a version-1
        // binary refuses it at the header check instead of mid-read.
        let required = entries
            .iter()
            .map(journal_version_required_by)
            .max()
            .unwrap_or(JOURNAL_VERSION);
        if required > journal.version {
            journal.bump_header(&mut entries, required)?;
        }
        Ok((journal, entries))
    }

    /// Atomically rewrite the journal in place with its header version set
    /// to `new_version` (sibling temp file, fsync, rename — the `compact`
    /// machinery), and update `entries` to match. A failure leaves the old
    /// journal untouched.
    fn bump_header(&mut self, entries: &mut [JournalEntry], new_version: u32) -> Result<()> {
        ensure!(
            matches!(entries.first(), Some(JournalEntry::Created { .. })),
            "world journal {} has no header to bump",
            self.path.display()
        );
        if let Some(JournalEntry::Created { version, .. }) = entries.first_mut() {
            *version = new_version;
        }
        self.compact(entries)?;
        self.version = new_version;
        Ok(())
    }
    /// Scan journal entries from `reader` (the recovery half of
    /// [`FileJournal::open`], split out so tests can drive it with
    /// bounded readers). Returns the entries, the byte length of the
    /// good prefix, and whether an unterminated torn tail was found.
    ///
    /// A malformed newline-terminated line is corruption and is
    /// reported WITHOUT reading the rest of the journal (#8728): the
    /// old code drained the remainder into memory first, so an early
    /// bad line in a large journal cost a whole-file allocation (or
    /// an unrelated I/O error) before the corruption was reported.
    /// An unterminated line is torn only when nothing follows it;
    /// that is probed with a single byte, never a drain.
    fn scan_entries<R: BufRead>(
        reader: &mut R,
        path: &Path,
    ) -> Result<(Vec<JournalEntry>, u64, bool)> {
        let mut entries = Vec::new();
        let mut good_len: u64 = 0;
        let mut torn = false;
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            let complete = line.last() == Some(&b'\n');
            if line == b"\n" {
                // Empty lines carry no entry. Preserve their bytes so a
                // later torn-tail repair cannot truncate past them.
                good_len += read as u64;
                continue;
            }
            match serde_json::from_slice::<JournalEntry>(&line) {
                Ok(entry) if complete => {
                    entries.push(entry);
                    good_len += read as u64;
                }
                parsed => {
                    // An unterminated last line is a torn append: the crash
                    // came before the entry was acknowledged. A newline-
                    // terminated line that does not parse was fully written,
                    // so it is corruption or an entry from a newer binary,
                    // and it must never be dropped silently, even as the
                    // last line.
                    let corruption = || {
                        anyhow::anyhow!(
                            "world journal {} is corrupt at byte {good_len}: {}",
                            path.display(),
                            parsed
                                .err()
                                .map_or_else(|| "unterminated entry".to_owned(), |e| e.to_string())
                        )
                    };
                    if complete {
                        return Err(corruption());
                    }
                    let mut probe = [0u8; 1];
                    if reader.read(&mut probe)? > 0 {
                        return Err(corruption());
                    }
                    torn = true;
                    break;
                }
            }
        }
        Ok((entries, good_len, torn))
    }

    /// Encoded size of `entries` as the journal holds them: one JSON
    /// object per line, each newline-terminated — exactly what
    /// `compact` writes. Lets a transaction be measured against the
    /// byte cap BEFORE its rewrite, so a refusal writes nothing
    /// (#10730).
    fn encoded_journal_len(entries: &[JournalEntry]) -> Result<u64> {
        let mut total: u64 = 0;
        for entry in entries {
            let len = serde_json::to_vec(entry)
                .context("encode journal entry")?
                .len() as u64;
            total += len + 1;
        }
        Ok(total)
    }

    /// The acknowledgement mode this journal was opened with (see
    /// [`JournalAckMode`]).
    #[must_use]
    pub fn ack_mode(&self) -> JournalAckMode {
        self.ack_mode
    }

    /// Appends acknowledged since the last completed sync — exactly
    /// the acknowledged history an OS crash could still lose. Always
    /// zero in [`JournalAckMode::Sync`].
    #[must_use]
    pub fn unsynced_appends(&self) -> u64 {
        self.unsynced_appends
    }

    /// Write one already-encoded append payload (`appends` counts the
    /// entries in it: 1 for `append`, the batch length for
    /// `append_batch`) and treat it per this journal's ack mode: sync
    /// it durable before returning ([`JournalAckMode::Sync`]), or
    /// write it and count it toward the next sync point (the relaxed
    /// modes), flushing when a `Batched` batch fills.
    fn append_write(&mut self, payload: &[u8], appends: u64) -> std::io::Result<()> {
        match self.ack_mode {
            JournalAckMode::Sync => sync_journal_write(&mut self.file, payload),
            JournalAckMode::Batched | JournalAckMode::Async => {
                write_journal_line(&mut self.file, payload)?;
                self.unsynced_appends += appends;
                if self.ack_mode == JournalAckMode::Batched
                    && self.unsynced_appends >= JOURNAL_ACK_BATCH_SIZE
                {
                    self.sync_durable()?;
                }
                Ok(())
            }
        }
    }

    /// Sync the journal file if any acknowledged append is still
    /// uncovered; on success every append so far is durable and
    /// [`Self::unsynced_appends`] is zero. A failed sync leaves the
    /// count standing — the appends are still uncovered — and reports
    /// the error, so the caller fails the current append rather than
    /// acknowledging past a sync point that did not happen.
    fn sync_durable(&mut self) -> std::io::Result<()> {
        if self.unsynced_appends == 0 {
            return Ok(());
        }
        sync_journal_file(&mut self.file)?;
        self.unsynced_appends = 0;
        Ok(())
    }
}

impl WorldJournal for FileJournal {
    fn ack_mode(&self) -> JournalAckMode {
        self.ack_mode
    }

    fn sync(&mut self) -> Result<()> {
        self.sync_durable().context("sync world journal")
    }

    fn append(&mut self, entry: &JournalEntry) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
        // Bump-on-first-write (issue #227): before appending the first
        // entry a version-1 binary cannot read, atomically rewrite the
        // journal under the header version that entry requires, so the
        // old binary refuses the journal at its header check and never
        // reaches the entry mid-read.
        let required = journal_version_required_by(entry);
        if required > self.version {
            let mut entries = read_journal_entries(&self.path)?;
            self.bump_header(&mut entries, required)?;
        }
        let mut line = serde_json::to_vec(entry).context("encode journal entry")?;
        line.push(b'\n');
        // F12.18 journal-size bound (#560, option B): refuse the append
        // BEFORE writing when it would cross the byte cap, so the file
        // never exceeds the cap and no partial line lands.
        let new_bytes = self.bytes + line.len() as u64;
        ensure!(
            new_bytes <= self.max_bytes,
            "world journal {} would exceed its byte cap ({new_bytes} > {} bytes); append refused",
            self.path.display(),
            self.max_bytes
        );
        self.append_write(&line, 1)
            .context("write world journal append")?;
        self.bytes = new_bytes;
        Ok(())
    }

    fn append_batch(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
        if entries.is_empty() {
            return Ok(());
        }
        // One sync covers the whole batch (#8239) - unless an entry needs
        // a header version bump, whose rewrite-then-append dance is the
        // per-entry path's job; fall back to it for the whole batch so
        // ordering and bump semantics stay exactly the single-append ones.
        if entries
            .iter()
            .any(|entry| journal_version_required_by(entry) > self.version)
        {
            for entry in entries {
                self.append(entry)?;
            }
            return Ok(());
        }
        let mut batch = Vec::new();
        for entry in entries {
            batch.extend_from_slice(&serde_json::to_vec(entry).context("encode journal entry")?);
            batch.push(b'\n');
        }
        // The byte cap is checked on the batch total BEFORE writing, so a
        // refusal writes nothing - never a prefix of the batch.
        let new_bytes = self.bytes + batch.len() as u64;
        ensure!(
            new_bytes <= self.max_bytes,
            "world journal {} would exceed its byte cap ({new_bytes} > {} bytes); append refused",
            self.path.display(),
            self.max_bytes
        );
        // The concatenated batch is ONE journal write: in Sync mode a
        // single sync_data makes it durable as a whole; the relaxed
        // modes count its entries toward their next sync point.
        self.append_write(&batch, entries.len() as u64)
            .context("write world journal batch")?;
        self.bytes = new_bytes;
        Ok(())
    }

    fn replication_snapshot(&mut self) -> Result<crate::world::JournalSnapshot> {
        let bytes = std::fs::read(&self.path)
            .with_context(|| format!("read world journal snapshot {}", self.path.display()))?;
        ensure!(
            bytes.is_empty() || bytes.ends_with(b"\n"),
            "world journal {} has an unterminated final entry",
            self.path.display()
        );
        Ok(crate::world::JournalSnapshot {
            header_version: self.version,
            bytes,
        })
    }

    fn replication_snapshot_source(&mut self) -> Result<crate::world::JournalSnapshotSource> {
        // Open a separate descriptor while the actor still owns the journal.
        // It pins the current inode across compaction, and the captured length
        // excludes appends that happen after the actor returns to its loop.
        let file = File::open(&self.path)
            .with_context(|| format!("open world journal snapshot {}", self.path.display()))?;
        let byte_len = file
            .metadata()
            .with_context(|| format!("stat world journal snapshot {}", self.path.display()))?
            .len();
        Ok(crate::world::JournalSnapshotSource::from_reader(
            self.version,
            byte_len,
            file,
        ))
    }

    fn replication_header_version(&self) -> Option<u32> {
        Some(self.version)
    }

    fn append_transaction(&mut self, transaction: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
        ensure!(!transaction.is_empty(), "empty world journal transaction");
        let mut entries = read_journal_entries(&self.path)?;
        ensure!(
            matches!(entries.first(), Some(JournalEntry::Created { .. })),
            "world journal {} has no header for transaction",
            self.path.display()
        );
        let required = transaction
            .iter()
            .map(journal_version_required_by)
            .max()
            .unwrap_or(self.version)
            .max(self.version);
        if let Some(JournalEntry::Created { version, .. }) = entries.first_mut() {
            *version = required;
        }
        entries.extend(transaction.iter().cloned());
        // F12.18 journal-size bound (#560, option B), transaction path
        // (#10730): the rewrite installs `entries`, so the size that must
        // fit is the encoded size of that whole journal — measured BEFORE
        // the rewrite, so a refusal writes nothing and never a prefix of
        // the transaction. Only GROWTH past the cap refuses: a journal may
        // already be over its cap (an older journal, a legacy oversized
        // entry), and the shrink paths — a checkpoint trim and the
        // bump-on-first-write header rewrite — must keep working.
        let new_bytes = Self::encoded_journal_len(&entries)?;
        ensure!(
            new_bytes <= self.max_bytes || new_bytes <= self.bytes,
            "world journal {} would exceed its byte cap ({new_bytes} > {} bytes); transaction refused",
            self.path.display(),
            self.max_bytes
        );
        // `compact` builds and syncs a complete sibling journal before a
        // single rename, so a failed transaction leaves the previous journal
        // intact and a crash sees either all entries or none of them.
        self.compact(&entries)
    }

    fn compact(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
        // One lifetime-held lock per journal path makes concurrent compaction
        // of the same journal unreachable; the sibling temp can therefore be
        // shared safely between sequential compactions.
        let tmp = self.path.with_extension("jsonl.tmp");
        // Build the replacement beside the journal; the live file is untouched
        // until the rename, so any failure before it leaves the old journal.
        let built = (|| -> Result<(File, u64)> {
            let mut options = OpenOptions::new();
            options.read(true).append(true).create(true).truncate(false);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            file.set_len(0)?;
            let mut encoded = Vec::new();
            for entry in entries {
                serde_json::to_writer(&mut encoded, entry).context("encode journal entry")?;
                encoded.push(b'\n');
            }
            file.write_all(&encoded)
                .context("write compacted journal")?;
            file.sync_all().context("sync compacted journal")?;
            Ok((file, encoded.len() as u64))
        })();
        let (file, compacted_bytes) = match built {
            Ok(pair) => pair,
            Err(error) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(error);
            }
        };
        if let Err(error) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::Error::from(error).context("replace world journal"));
        }
        // The new file is now the journal, and the open handle follows its
        // inode. If the directory entry cannot be made durable, a crash could
        // resurrect the old journal and lose later appends: refuse them.
        self.file = file;
        let synced = self.path.parent().map_or(Ok(()), |parent| {
            File::open(parent).and_then(|dir| dir.sync_all())
        });
        if let Err(error) = synced {
            self.broken = true;
            return Err(anyhow::Error::from(error).context("sync world directory"));
        }
        if let Some(JournalEntry::Created { version, .. }) = entries.first() {
            self.version = *version;
        }
        // The rewrite IS the journal now: re-baseline the tracked size
        // (this is how a checkpoint trim brings an over-cap journal back
        // under its cap, and how transactions account for their bytes).
        self.bytes = compacted_bytes;
        // The replacement was built, synced and renamed before this
        // point, so every entry it holds is durable: a compaction is a
        // sync point in every ack mode.
        self.unsynced_appends = 0;
        Ok(())
    }
}

/// Content-addressed module files for one world.
#[derive(Clone, Debug)]
pub struct ModuleDir {
    dir: PathBuf,
}

impl ModuleDir {
    /// Use `dir` (created on first save).
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, hash: &str) -> Result<PathBuf> {
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid module hash"
        );
        Ok(self.dir.join(format!("{hash}.wasm")))
    }

    /// Durably store `wasm` and return its hash. Writes to a temporary file
    /// and renames, so a crash never leaves a partial module under its hash.
    ///
    /// # Errors
    ///
    /// I/O failures.
    pub fn save(&self, wasm: &[u8]) -> Result<String> {
        let hash = module_hash(wasm);
        let path = self.path(&hash)?;
        if path.exists() {
            return Ok(hash);
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create {}", self.dir.display()))?;
        let tmp = self.dir.join(format!(".{hash}.tmp"));
        {
            let mut file = File::create(&tmp)?;
            file.write_all(wasm)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &path)?;
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(hash)
    }

    /// Load a module and verify it still matches its hash.
    ///
    /// The stored file for `hash` (a native worker needs the path).
    pub(crate) fn path_of(&self, hash: &str) -> Result<PathBuf> {
        self.path(hash)
    }

    /// Read the module stored under `hash`, verifying its hash.
    ///
    /// # Errors
    ///
    /// Missing file or a hash mismatch (tampering or disk corruption).
    pub fn load(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.path(hash)?;
        let wasm = std::fs::read(&path)
            .with_context(|| format!("read world module {}", path.display()))?;
        ensure!(
            module_hash(&wasm) == hash,
            "world module {} does not match its hash",
            path.display()
        );
        Ok(wasm)
    }
}

/// Stands in for a program that could not be restored; every call fails,
/// which makes the actor mark the program unhealthy until an admin installs
/// a replacement. Chat and membership keep working.
struct UnavailableProgram(String);

impl WorldProgram for UnavailableProgram {
    fn update(&mut self, _event: &crate::world::WorldEvent) -> Result<()> {
        bail!("world program unavailable: {}", self.0)
    }
    fn view(&mut self, _viewer: &crate::world::Participant) -> Result<Option<String>> {
        bail!("world program unavailable: {}", self.0)
    }
}

/// What [`open_world`] found on disk.
#[derive(Clone, Debug, Default)]
pub struct OpenReport {
    /// Last committed sequence.
    pub sequence: u64,
    /// Participants (including revoked) known to the journal.
    pub journal_entries: usize,
    /// Active module, if any.
    pub module_hash: Option<String>,
    /// Inputs replayed into the restored program.
    pub replayed_inputs: usize,
    /// Why the program could not be restored, if it could not.
    pub program_error: Option<String>,
}

/// Open a durable world rooted at `dir`, restoring state from its journal.
///
/// # Errors
///
/// Fails if the journal is corrupt or belongs to another world. A module
/// that cannot be loaded or replayed does **not** fail the open: the world
/// starts with chat and membership intact and the program marked unavailable
/// (see [`OpenReport::program_error`]).
pub async fn open_world(
    dir: &Path,
    world_id: &str,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    service: impl FnOnce(WorldHandle) -> WorldService,
) -> Result<(WorldService, OpenReport)> {
    open_world_inner(dir, world_id, None, limits, runtime, None, service).await
}

/// Open a durable world and install its runtime account and world-membership
/// boundary gate before the restored core is moved into the world actor.
///
/// The gate is runtime wiring and is deliberately not journaled. Callers
/// without an operator boundary configuration should use [`open_world`].
pub async fn open_world_with_directory_gate(
    dir: &Path,
    world_id: &str,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    gate: Option<std::sync::Arc<dyn id_directory::directory::BoundaryGate>>,
    service: impl FnOnce(WorldHandle) -> WorldService,
) -> Result<(WorldService, OpenReport)> {
    open_world_inner(dir, world_id, None, limits, runtime, gate, service).await
}

/// Open a durable world inside one partition (#3898): the journal is
/// opened through [`FileJournal::open_in_partition`], so a new world is
/// bound to `jurisdiction` before its first journal byte, and a world
/// bound elsewhere, an unassigned legacy world or a malformed binding is
/// refused before the journal is read and before its directory, petname
/// or outbox files are touched.
///
/// `jurisdiction` must come from trusted server or account configuration,
/// never from a client request.
///
/// # Errors
///
/// As [`open_world_with_directory_gate`], plus every refusal of
/// [`FileJournal::open_in_partition`].
pub async fn open_world_in_partition(
    dir: &Path,
    world_id: &str,
    jurisdiction: &str,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    gate: Option<std::sync::Arc<dyn id_directory::directory::BoundaryGate>>,
    service: impl FnOnce(WorldHandle) -> WorldService,
) -> Result<(WorldService, OpenReport)> {
    open_world_inner(
        dir,
        world_id,
        Some(jurisdiction),
        limits,
        runtime,
        gate,
        service,
    )
    .await
}

async fn open_world_inner(
    dir: &Path,
    world_id: &str,
    jurisdiction: Option<&str>,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    gate: Option<std::sync::Arc<dyn id_directory::directory::BoundaryGate>>,
    service: impl FnOnce(WorldHandle) -> WorldService,
) -> Result<(WorldService, OpenReport)> {
    let journal_path = dir.join("journal.jsonl");
    let id = world_id.to_owned();
    let ack_mode = limits.journal_ack_mode;
    let jurisdiction = jurisdiction.map(str::to_owned);
    let (journal, entries) = tokio::task::spawn_blocking(move || match jurisdiction {
        Some(jurisdiction) => {
            FileJournal::open_in_partition(&journal_path, &id, &jurisdiction, ack_mode)
        }
        None => FileJournal::open_with_ack_mode(&journal_path, &id, ack_mode),
    })
    .await
    .context("journal open task")??;
    let journal_entries = entries.len();
    let mut restored = WorldCore::restore_with_runtime(world_id, limits, runtime, entries)?;
    let directory_path = dir.join("directory.jsonl");
    let directory =
        tokio::task::spawn_blocking(move || crate::directory::Directory::open(&directory_path))
            .await
            .context("directory open task")??;
    *restored.core.directory_mut() = directory;
    install_world_boundary_gate(&mut restored.core, gate);
    let petnames_path = dir.join("petnames.jsonl");
    let petnames =
        tokio::task::spawn_blocking(move || crate::petname::Petnames::open(&petnames_path))
            .await
            .context("petnames open task")??;
    restored.core.set_petnames(petnames);
    let outbox_path = dir.join("outbox.jsonl");
    let outbox =
        tokio::task::spawn_blocking(move || crate::envelope_outbox::Outbox::open(&outbox_path))
            .await
            .context("outbox open task")??;
    let modules = ModuleDir::new(dir.join("modules"));
    let mut report = OpenReport {
        sequence: 0,
        journal_entries,
        module_hash: restored.program.as_ref().map(|p| p.module_hash.clone()),
        replayed_inputs: 0,
        program_error: None,
    };

    let mut restored_module = None;
    let program: Box<dyn WorldProgram> = match &restored.program {
        None => Box::new(EmptyProgram),
        Some(spec) => match restore_program(
            &modules,
            world_id,
            spec,
            &restored.replay,
            restored.core.execution_profile().resolve(runtime)?,
        )
        .await
        {
            Ok((program, wasm)) => {
                report.replayed_inputs = restored.replay.len();
                restored_module = Some(wasm);
                program
            }
            Err(error) => {
                let message = format!("{error:#}");
                tracing::error!("world {world_id}: could not restore program: {message}");
                report.program_error = Some(message.clone());
                Box::new(UnavailableProgram(message))
            }
        },
    };
    report.sequence = restored.core.current_sequence();
    let handle = WorldHandle::spawn_restored(
        restored.core,
        program,
        Some(Box::new(journal)),
        Some(WorldHandle::host_device_manifest()),
    );
    let service = service(handle).with_module_dir(modules).with_outbox(outbox);
    if let Some(wasm) = restored_module {
        service.adopt_module(wasm).await?;
    }
    Ok((service, report))
}

fn install_world_boundary_gate(
    core: &mut WorldCore,
    gate: Option<std::sync::Arc<dyn id_directory::directory::BoundaryGate>>,
) {
    if let Some(gate) = gate {
        core.directory_mut()
            .set_boundary_gate(std::sync::Arc::clone(&gate));
        core.set_world_boundary_gate(gate);
    }
}

/// Open a read-only replica world rooted at `dir` (F12.13 phase 4).
///
/// The replica directory holds the verified journal at
/// `journal.jsonl` (written by [`crate::world::DurableReceiver`]), an
/// optional `directory.jsonl` carrying the directory facts the replicated
/// capabilities depend on (replayed without rewriting it; installed by
/// the receiver from the stream's signed Directory sidecar frames), and
/// an optional `modules/` store (populated by the receiver from signed
/// Module sidecar frames, content-addressed by hash). Opening never invents history: the committed journal
/// entries are re-validated and restored presence-preserving (no header
/// bumps, no entry synthesis, no compaction), and the actor spawns
/// read-only with no journal to append to — every local mutation and
/// program side effect is refused while spectator reads serve over the
/// existing session protocol.
///
/// Crash debris is recovered, not served: like the receiver, opening drops
/// an interrupted tail append back to the last committed batch and ignores
/// torn scratch files. Either way the last committed state is what serves.
///
/// # Errors
///
/// Fails when no committed replica journal exists yet, when the journal is
/// not a well-formed chain for `world_id` under `pinned_source`, or when
/// the directory facts do not apply. A module that cannot be loaded or
/// replayed does **not** fail the open: like the writer path, the replica
/// serves with the program marked unavailable (see
/// [`OpenReport::program_error`]).
pub async fn open_replica(
    dir: &Path,
    world_id: &str,
    pinned_source: [u8; 32],
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    service: impl FnOnce(WorldHandle) -> WorldService,
) -> Result<(WorldService, OpenReport, crate::world::ReplicaCursor)> {
    let (service, report, cursor, _status) = open_replica_impl(
        dir,
        world_id,
        pinned_source,
        limits,
        runtime,
        service,
        None,
        0,
    )
    .await?;
    Ok((service, report, cursor))
}

/// Operator-facing status of an opened replica (F12.13 phase 5): the
/// pinned source it follows, the world it serves, where its verified
/// cursor stands, and how much of the policy-B admission window
/// remains — the staleness surface an operator (or a follow CLI)
/// reports from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaStatus {
    /// The world this replica serves.
    pub world_id: String,
    /// The pinned writer source key the journal was verified against.
    pub source: [u8; 32],
    /// The verified journal cursor the replica stands at.
    pub cursor: crate::world::ReplicaCursor,
    /// When the held admission lease expires, if the replica was
    /// opened under one.
    pub lease_expires_at_unix_ms: Option<u64>,
    /// Admission window left at open time, in milliseconds (zero once
    /// expired); `None` when the replica was opened ungated.
    pub lease_remaining_ms: Option<u64>,
    /// Whether a NEW spectator may be admitted at open time: under a
    /// gate, only while the held lease verifies; ungated replicas keep
    /// the journal-only admission of the phase-4 path.
    pub admits_new_spectators: bool,
}

/// As [`open_replica`], under the policy-B admission gate (F12.13
/// phase 5): the replica serves reads exactly as before, but a
/// spectator who was not present at the verified cursor is admitted
/// only while `lease` — the writer-signed admission lease for this
/// world from the pinned source — verifies at admission time
/// ([`crate::world::SignedAdmissionLease::verify`], enforced by
/// [`crate::world::ReplicaAdmissionGate`] in the actor). An expired
/// or not-yet-valid lease does NOT fail the open: serving continues
/// and new admissions stop until resync, per policy B. A lease that
/// fails verification for a non-time reason (another source, another
/// world, a bad signature) is an operator misconfiguration and fails
/// the open. A `None` lease opens the replica gated with no lease
/// held: existing participants are served, no new spectator is
/// admitted. `now_unix_ms` is the open-time clock for the returned
/// [`ReplicaStatus`]; the gate itself checks the actor's clock at
/// each admission.
///
/// # Errors
///
/// As [`open_replica`], and additionally when a supplied lease fails
/// verification for a non-time reason.
pub async fn open_replica_with_admission(
    dir: &Path,
    world_id: &str,
    pinned_source: [u8; 32],
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    service: impl FnOnce(WorldHandle) -> WorldService,
    lease: Option<crate::world::SignedAdmissionLease>,
    now_unix_ms: u64,
) -> Result<(
    WorldService,
    OpenReport,
    crate::world::ReplicaCursor,
    ReplicaStatus,
)> {
    open_replica_impl(
        dir,
        world_id,
        pinned_source,
        limits,
        runtime,
        service,
        Some(lease),
        now_unix_ms,
    )
    .await
}

async fn open_replica_impl(
    dir: &Path,
    world_id: &str,
    pinned_source: [u8; 32],
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    service: impl FnOnce(WorldHandle) -> WorldService,
    admission: Option<Option<crate::world::SignedAdmissionLease>>,
    now_unix_ms: u64,
) -> Result<(
    WorldService,
    OpenReport,
    crate::world::ReplicaCursor,
    ReplicaStatus,
)> {
    use crate::world::{DurableReceiver, restore_replica_entries_with_runtime};

    // A lease bound to another source or world, or with a bad
    // signature, is a misconfiguration: fail before touching the
    // directory. Time-window failures are not: policy B keeps the
    // replica serving with new admissions stopped.
    if let Some(Some(lease)) = &admission {
        use crate::world::AdmissionLeaseError;
        match lease.verify(pinned_source, world_id, now_unix_ms) {
            Ok(()) | Err(AdmissionLeaseError::Expired) | Err(AdmissionLeaseError::NotYetValid) => {}
            Err(error) => return Err(error.into()),
        }
    }

    let dir_owned = dir.to_owned();
    let id = world_id.to_owned();
    let id_for_receiver = id.clone();
    let (journal_bytes, committed) = tokio::task::spawn_blocking(move || {
        let receiver = DurableReceiver::open(&dir_owned, &id_for_receiver, pinned_source)?;
        let committed = receiver.committed_cursor();
        let bytes = receiver.read_journal()?;
        Ok::<_, crate::world::ReceiverError>((bytes, committed))
    })
    .await
    .context("replica open task")??;
    ensure!(
        committed.is_some(),
        "replica {world_id} has no committed journal yet"
    );
    let entries = crate::world::parse_replica_journal(&journal_bytes, world_id)?;
    let journal_entries = entries.len();
    let subjects = replica_subjects(&entries);
    let directory_path = dir.join("directory.jsonl");
    let restored = tokio::task::spawn_blocking({
        let directory_path = directory_path.clone();
        move || {
            // Read-only replay: opening a replica must not truncate or
            // rewrite the directory sidecar.
            let directory = crate::directory::Directory::replay_read_only(&directory_path)
                .context("directory open")?;
            let restored = restore_replica_entries_with_runtime(
                &id, limits, runtime, entries, &directory, &subjects,
            )?;
            Ok::<_, anyhow::Error>(restored)
        }
    })
    .await
    .context("replica restore task")??;
    let core = restored.core;
    // Petnames are explicitly outside F12.13's replication scope, and no
    // local directory session can run in replica mode. Keep the default
    // in-memory model instead of opening a mutating sidecar.
    let cursor = restored.cursor;
    let modules = ModuleDir::new(dir.join("modules"));
    let mut report = OpenReport {
        sequence: cursor.sequence,
        journal_entries,
        module_hash: restored.program.as_ref().map(|p| p.module_hash.clone()),
        replayed_inputs: 0,
        program_error: None,
    };

    let mut restored_module = None;
    let program: Box<dyn WorldProgram> = match &restored.program {
        None => Box::new(EmptyProgram),
        Some(spec) => match restore_program(
            &modules,
            world_id,
            spec,
            &restored.replay,
            core.execution_profile().resolve(runtime)?,
        )
        .await
        {
            Ok((program, wasm)) => {
                report.replayed_inputs = restored.replay.len();
                restored_module = Some(wasm);
                program
            }
            Err(error) => {
                let message = format!("{error:#}");
                tracing::error!("replica {world_id}: could not restore program: {message}");
                report.program_error = Some(message.clone());
                Box::new(UnavailableProgram(message))
            }
        },
    };
    // No journal, no outbox: the replica appends nothing and queues nothing
    // for other servers. Directory writes are refused by the read-only
    // actor, so directory facts stay fixed to the local replica sidecar.
    let (lease_expires_at_unix_ms, lease_remaining_ms, admits_new_spectators) = match &admission {
        None => (None, None, true),
        Some(None) => (None, None, false),
        Some(Some(lease)) => (
            Some(lease.lease.expires_at_unix_ms),
            Some(lease.remaining_ms(now_unix_ms)),
            lease.admits_new_spectator(pinned_source, world_id, now_unix_ms),
        ),
    };
    let admission_gate = admission.map(|lease| {
        crate::world::ReplicaAdmissionGate::new(
            lease,
            pinned_source,
            world_id,
            restored.presence.keys().copied().collect(),
        )
    });
    let handle = match admission_gate {
        Some(gate) => WorldHandle::spawn_read_only_with_admission(
            core,
            program,
            restored.presence,
            Some(gate),
        )?,
        None => WorldHandle::spawn_read_only(core, program, restored.presence)?,
    };
    let mut service = service(handle).with_module_dir(modules);
    // Replica sessions are spectator-only; a configured writer admin token,
    // mail sender, compiler, outbox or blob uploader must not leak into the
    // replica service. The actor remains the authoritative enforcement layer.
    service.admin_token = None;
    service.mail = None;
    service.outbox = None;
    service.compiler = None;
    service.blobs = None;
    if let Some(wasm) = restored_module {
        service.adopt_module(wasm).await?;
    }
    let status = ReplicaStatus {
        world_id: world_id.to_owned(),
        source: pinned_source,
        cursor,
        lease_expires_at_unix_ms,
        lease_remaining_ms,
        admits_new_spectators,
    };
    Ok((service, report, cursor, status))
}

/// Every account appearing in a replica journal's capability chains: the
/// subjects the replica's directory facts must cover for
/// [`WorldCore::effective_scopes`](crate::world::WorldCore::effective_scopes).
fn replica_subjects(entries: &[JournalEntry]) -> std::collections::BTreeSet<String> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Issued {
                subject: Some(subject),
                ..
            } => Some(subject.clone()),
            _ => None,
        })
        .collect()
}

/// What a fork copies from one world into a new one (F12.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkSpec {
    /// The world the fork is copied from.
    pub from_world_id: String,
    /// The new world's id (what its journal header will say).
    pub to_world_id: String,
    /// Whether the fork is marked reusable as a template.
    pub template: bool,
}

/// Read a journal's entries without opening it for append.
///
/// A torn final fragment — a last chunk with no terminating newline, left by
/// a crash mid-append — is excluded with a warning, mirroring
/// [`FileJournal::open`]: that entry was never acknowledged, and a world its
/// own open repairs must stay forkable (#1523). A newline-terminated line
/// that does not parse was fully written, so it stays fatal corruption in
/// any position, including the last line.
fn read_journal_entries(path: &Path) -> Result<Vec<JournalEntry>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if bytes.last() != Some(&b'\n') {
        // The split's final chunk is an unterminated fragment (or the empty
        // tail of an empty file); it is not a complete entry.
        if let Some(fragment) = lines.pop() {
            if !fragment.is_empty() {
                tracing::warn!(
                    "world journal {}: dropping torn final entry at byte {}",
                    path.display(),
                    bytes.len() - fragment.len()
                );
            }
        }
    }
    let mut entries = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let entry: JournalEntry = serde_json::from_slice(line)
            .with_context(|| format!("{} is corrupt", path.display()))?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Fork a durable world: copy the source's journal and modules into `dst_dir`
/// under a new world id, and record the fork provenance (F12.2).
///
/// The copied entries reproduce the parent's state exactly — the fork replays
/// to the parent's records at the fork point. The `Created` header is
/// rewritten to the new id, a [`JournalEntry::Forked`] records the origin, and
/// the module files are copied unchanged.
///
/// # Errors
///
/// A missing or corrupt source journal, a source journal that belongs to a
/// different world, or I/O failures writing the destination.
pub fn fork_world(src_dir: &Path, dst_dir: &Path, spec: &ForkSpec) -> Result<()> {
    let mut entries = read_journal_entries(&src_dir.join("journal.jsonl"))?;
    let Some(JournalEntry::Created {
        world_id,
        version: source_version,
    }) = entries.first()
    else {
        bail!("source journal has no Created header");
    };
    let source_version = *source_version;
    ensure!(
        world_id == &spec.from_world_id,
        "source journal belongs to {world_id:?}, not {:?}",
        spec.from_world_id
    );

    // The fork point: the latest checkpoint or program install.
    let mut at_sequence = 0u64;
    let mut from_module_hash: Option<String> = None;
    for entry in &entries {
        match entry {
            JournalEntry::Checkpoint {
                sequence,
                module_hash,
                ..
            } => {
                at_sequence = *sequence;
                from_module_hash = Some(module_hash.clone());
            }
            JournalEntry::Committed { event } => {
                at_sequence = event.sequence.max(at_sequence);
                if let WorldEventKind::ProgramInstalled { module_hash, .. } = &event.kind {
                    from_module_hash = Some(module_hash.clone());
                }
            }
            _ => {}
        }
    }

    // Rewrite the header to the new world and record the origin. The fork
    // keeps at least the source's header version (and whatever its entries
    // require): resetting it to `JOURNAL_VERSION` would re-create the
    // issue-#227 hazard for a source that already carries a profile entry.
    let fork_version = entries
        .iter()
        .map(journal_version_required_by)
        .max()
        .unwrap_or(JOURNAL_VERSION)
        .max(source_version);
    entries[0] = JournalEntry::Created {
        world_id: spec.to_world_id.clone(),
        version: fork_version,
    };
    entries.push(JournalEntry::Forked {
        from_world_id: spec.from_world_id.clone(),
        from_module_hash: from_module_hash.unwrap_or_default(),
        template: spec.template,
        at_sequence,
    });

    // Write the destination journal, fsynced.
    std::fs::create_dir_all(dst_dir).with_context(|| format!("create {}", dst_dir.display()))?;
    let path = dst_dir.join("journal.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    for entry in &entries {
        file.write_all(serde_json::to_string(entry)?.as_bytes())?;
        file.write_all(b"\n")?;
    }
    file.sync_all()?;

    // Copy the module files, content-addressed under the same hashes.
    let src_modules = src_dir.join("modules");
    if src_modules.is_dir() {
        let dst_modules = dst_dir.join("modules");
        std::fs::create_dir_all(&dst_modules)
            .with_context(|| format!("create {}", dst_modules.display()))?;
        for entry in std::fs::read_dir(&src_modules)? {
            let entry = entry?;
            std::fs::copy(entry.path(), dst_modules.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Project a durable world on disk into a template source (#8282).
///
/// Reads `dir/journal.jsonl` as [`fork_world`] does, tolerating a torn
/// final entry, and hands the entries to
/// [`crate::world_template::template_source_from_journal`]. Only the
/// `kv` keys in `keep_state` become template state.
///
/// # Errors
///
/// A missing or corrupt journal, or any refusal of the projection.
pub fn template_source_from_dir(
    dir: &Path,
    keep_state: &[String],
) -> Result<crate::world_template::TemplateSource> {
    let entries = read_journal_entries(&dir.join("journal.jsonl"))?;
    Ok(crate::world_template::template_source_from_journal(
        &entries, keep_state,
    )?)
}

#[derive(Debug, Default)]
struct EmptyProgram;
impl WorldProgram for EmptyProgram {}

/// Restore a program of either tier. A native module (an ELF worker) is
/// spawned from its stored file; a Wasm module runs in the sandbox. Both
/// start from the checkpoint snapshot when the journal has one.
async fn restore_program(
    modules: &ModuleDir,
    world_id: &str,
    spec: &crate::world::RestoredProgram,
    replay: &[crate::world::WorldEvent],
    runtime: crate::world_limits::RuntimeLimits,
) -> Result<(Box<dyn WorldProgram>, Vec<u8>)> {
    let modules = modules.clone();
    let world_id = world_id.to_owned();
    let spec = spec.clone();
    let replay = replay.to_vec();
    tokio::task::spawn_blocking(move || {
        let seed = spec.seed;
        let wasm = modules.load(&spec.module_hash)?;
        let mut program: Box<dyn WorldProgram> = if crate::world_native::is_native_module(&wasm) {
            let path = modules.path_of(&spec.module_hash)?;
            Box::new(crate::world_native::NativeProgram::new(
                &path,
                seed,
                runtime.native,
                spec.snapshot.as_deref(),
            )?)
        } else {
            #[cfg(feature = "sandbox")]
            {
                let mut guest =
                    crate::sandbox::Sandbox::compile(&wasm, runtime.sandbox)?.instantiate(seed)?;
                // Charge this world's burns to its world id, against
                // the hub's configured burn budget (#1067). The
                // per-account bound and the admin account registry
                // ride along (#1067 per-account quotas): the bound
                // applies to the shared budget itself (0 = none, the
                // budget's own configuration point), and the guest
                // resolves its charge account against the registry
                // the serve path installed.
                guest.set_charge_key(world_id.clone());
                let burn_budget =
                    crate::guest_burn::GuestBurnBudget::shared_for(runtime.guest_burn_concurrency);
                if runtime.guest_burn_per_account_concurrency > 0 {
                    burn_budget.set_account_bound(Some(runtime.guest_burn_per_account_concurrency));
                }
                guest.set_burn_budget(burn_budget);
                guest.set_account_registry(crate::account_charge::WorldAccountRegistry::shared());
                if let Some(snapshot) = &spec.snapshot {
                    guest
                        .restore(snapshot.as_bytes())
                        .context("restore program from checkpoint")?;
                }
                Box::new(guest)
            }
            #[cfg(not(feature = "sandbox"))]
            anyhow::bail!("wasm modules need a build with the `sandbox` feature");
        };
        for event in &replay {
            program
                .update(event)
                .with_context(|| format!("replay input {}", event.sequence))?;
        }
        Ok((program, wasm))
    })
    .await
    .context("program restore task")?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::world::{Authority, CapabilityBounds, FileScope, Isolation, WorldScopes};
    use crate::world_limits::RuntimeLimits;
    use id_core::boundary::EnforcementPoint;
    use id_core::boundary_surfaces::FrameworkOutcome;
    use id_directory::directory::BoundaryGate;

    // #8728: the journal scan reports a malformed complete line
    // without reading the rest of the journal. The oracle is a
    // counting reader under a small-buffer BufReader: a scan that
    // drained the remainder (the old read_to_end) would serve every
    // byte of the tail; the bounded scan serves at most one buffer
    // past the malformed line.
    struct CountingRead {
        inner: std::io::Cursor<Vec<u8>>,
        served: usize,
    }

    impl std::io::Read for CountingRead {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.served += n;
            Ok(n)
        }
    }

    fn journal_line(entry: &JournalEntry) -> Vec<u8> {
        let mut line = serde_json::to_vec(entry).unwrap();
        line.push(b'\n');
        line
    }

    #[test]
    fn a_world_on_disk_projects_its_kept_state_into_a_template_source() {
        // Oracle: the store reads the journal as fork does, a torn
        // final entry included, and only the named kv keys come along.
        let dir = TempDir::new().unwrap();
        let mut bytes = Vec::new();
        for entry in [
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::KvSet {
                key: "layout".to_owned(),
                value: "1".to_owned(),
            },
            JournalEntry::KvSet {
                key: "scratch".to_owned(),
                value: "2".to_owned(),
            },
        ] {
            bytes.extend(journal_line(&entry));
        }
        bytes.extend_from_slice(b"{torn");
        std::fs::write(dir.path().join("journal.jsonl"), bytes).unwrap();

        let source = template_source_from_dir(dir.path(), &["layout".to_owned()]).unwrap();
        assert!(source.programs.is_empty());
        assert_eq!(
            source.state.into_iter().collect::<Vec<_>>(),
            vec![("layout".to_owned(), "1".to_owned())]
        );
        assert!(template_source_from_dir(dir.path(), &["absent".to_owned()]).is_err());
    }

    #[test]
    fn partition_fence_unscoped_open_refuses_before_tail_repair() {
        // Oracle: an assigned journal is inaccessible without trusted partition
        // context, and refusal preserves even an unacknowledged torn tail.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open(&path, "w").unwrap());
        std::fs::write(
            dir.path().join("journal.jsonl.partition.json"),
            br#"{"version":1,"world_id":"w","jurisdiction":"eu"}"#,
        )
        .unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{torn")
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(FileJournal::open(&path, "w").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn partition_fence_matching_reopen_can_append_foreign_cannot() {
        // Oracle: the creation binding survives handle drops and only that
        // partition can acquire an append-capable journal.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, _) =
            FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap();
        journal
            .append(&JournalEntry::CapsGranted {
                granted: vec!["a".into()],
            })
            .unwrap();
        drop(journal);
        let before = std::fs::read(&path).unwrap();
        for world_id in ["w", "other"] {
            assert!(
                FileJournal::open_in_partition(&path, world_id, "us", JournalAckMode::Sync)
                    .is_err()
            );
        }
        assert!(
            FileJournal::open_in_partition(&path, "other", "eu", JournalAckMode::Sync).is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let (mut journal, entries) =
            FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap();
        assert_eq!(entries.len(), 2);
        journal.compact(&entries).unwrap();
        journal
            .append(&JournalEntry::CapsGranted {
                granted: vec!["b".into()],
            })
            .unwrap();
        drop(journal);
        let (_, entries) =
            FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn partition_fence_never_adopts_legacy_or_creates_for_invalid_context() {
        // Oracle: explicit local/unassigned history is never relabelled; bad
        // trusted context refuses before any directory or journal side effect.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open(&path, "w").unwrap());
        let before = std::fs::read(&path).unwrap();
        assert!(FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("journal.jsonl.partition.json").exists());
        FileJournal::open(&path, "w").unwrap();
        let absent = dir.path().join("absent/journal.jsonl");
        for invalid in ["", "EU", "../eu", "eu/us"] {
            assert!(
                FileJournal::open_in_partition(&absent, "w", invalid, JournalAckMode::Sync)
                    .is_err()
            );
        }
        assert!(!absent.parent().unwrap().exists());
    }

    #[test]
    fn partition_fence_malformed_metadata_denies_without_repair() {
        // Oracle: corrupt/unknown-version bindings never downgrade to local.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap());
        let before = std::fs::read(&path).unwrap();
        for corrupt in ["{", r#"{"version":2,"world_id":"w","jurisdiction":"eu"}"#] {
            std::fs::write(dir.path().join("journal.jsonl.partition.json"), corrupt).unwrap();
            assert!(FileJournal::open(&path, "w").is_err());
            assert!(
                FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn partition_fence_production_world_open_cannot_bypass_binding() {
        // Oracle: the production service opener inherits the same fence and
        // never reaches restoration/directory/outbox creation for a scoped world.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap());
        let result = open_world(
            dir.path(),
            "w",
            WorldLimits::default(),
            RuntimeLimits::default(),
            |handle| WorldService::new(handle, None),
        )
        .await;
        assert!(result.is_err());
        assert!(!dir.path().join("directory.jsonl").exists());
        assert!(!dir.path().join("outbox.jsonl").exists());
    }

    #[tokio::test]
    async fn partition_scoped_world_open_binds_and_refuses_other_scopes() {
        // Oracle: a world created through the scoped opener is bound to its
        // partition, reopens there, and every other scope (another partition
        // or the unscoped opener) is refused before directory/outbox files
        // are created.
        let dir = TempDir::new().unwrap();
        let open = |jurisdiction: &'static str| {
            open_world_in_partition(
                dir.path(),
                "w",
                jurisdiction,
                WorldLimits::default(),
                RuntimeLimits::default(),
                None,
                |handle| WorldService::new(handle, None),
            )
        };
        let (service, _) = open("eu").await.unwrap();
        service.world().shutdown().await.unwrap();
        drop(service);
        assert!(world_bound_to_partition(dir.path(), "w", "eu"));
        assert!(!world_bound_to_partition(dir.path(), "w", "us"));
        assert!(!world_bound_to_partition(dir.path(), "other", "eu"));
        assert!(open("us").await.is_err());
        assert!(
            open_world(
                dir.path(),
                "w",
                WorldLimits::default(),
                RuntimeLimits::default(),
                |handle| WorldService::new(handle, None),
            )
            .await
            .is_err()
        );
        let (service, _) = open("eu").await.unwrap();
        service.world().shutdown().await.unwrap();

        let fresh = TempDir::new().unwrap();
        assert!(
            open_world_in_partition(
                fresh.path(),
                "w",
                "EU",
                WorldLimits::default(),
                RuntimeLimits::default(),
                None,
                |handle| WorldService::new(handle, None),
            )
            .await
            .is_err()
        );
        assert!(!fresh.path().join("directory.jsonl").exists());
        assert!(!fresh.path().join("journal.jsonl").exists());
    }

    #[tokio::test]
    async fn partition_scoped_world_open_never_adopts_a_legacy_world() {
        // Oracle: an unassigned local world is invisible to a partition and
        // is not relabelled by a scoped open; it still opens unscoped.
        let dir = TempDir::new().unwrap();
        let (service, _) = open_world(
            dir.path(),
            "w",
            WorldLimits::default(),
            RuntimeLimits::default(),
            |handle| WorldService::new(handle, None),
        )
        .await
        .unwrap();
        service.world().shutdown().await.unwrap();
        drop(service);
        assert!(!world_bound_to_partition(dir.path(), "w", "eu"));
        assert!(
            open_world_in_partition(
                dir.path(),
                "w",
                "eu",
                WorldLimits::default(),
                RuntimeLimits::default(),
                None,
                |handle| WorldService::new(handle, None),
            )
            .await
            .is_err()
        );
        assert!(!dir.path().join("journal.jsonl.partition.json").exists());
        let (service, _) = open_world(
            dir.path(),
            "w",
            WorldLimits::default(),
            RuntimeLimits::default(),
            |handle| WorldService::new(handle, None),
        )
        .await
        .unwrap();
        service.world().shutdown().await.unwrap();
    }

    #[test]
    fn partition_visibility_treats_bad_bindings_as_absent() {
        // Oracle: discovery never reveals a world through a malformed,
        // aliased or missing binding; only an exact v1 record is visible.
        let dir = TempDir::new().unwrap();
        assert!(!world_bound_to_partition(dir.path(), "w", "eu"));
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).unwrap());
        assert!(world_bound_to_partition(dir.path(), "w", "eu"));
        let sidecar = dir.path().join("journal.jsonl.partition.json");
        for corrupt in [
            "{",
            r#"{"version":2,"world_id":"w","jurisdiction":"eu"}"#,
            r#"{"version":1,"world_id":"w","jurisdiction":"eu","extra":1}"#,
        ] {
            std::fs::write(&sidecar, corrupt).unwrap();
            assert!(!world_bound_to_partition(dir.path(), "w", "eu"));
        }
        std::fs::write(
            &sidecar,
            br#"{"version":1,"world_id":"w","jurisdiction":"eu"}"#,
        )
        .unwrap();
        assert!(world_bound_to_partition(dir.path(), "w", "eu"));
        std::fs::remove_file(&path).unwrap();
        assert!(!world_bound_to_partition(dir.path(), "w", "eu"));
        #[cfg(unix)]
        {
            let real = TempDir::new().unwrap();
            let real_path = real.path().join("journal.jsonl");
            drop(
                FileJournal::open_in_partition(&real_path, "w", "eu", JournalAckMode::Sync)
                    .unwrap(),
            );
            std::os::unix::fs::symlink(&real_path, &path).unwrap();
            assert!(!world_bound_to_partition(dir.path(), "w", "eu"));
        }
    }

    #[test]
    fn partition_fence_concurrent_creation_has_one_binding() {
        // Oracle: exactly one of two competing first jurisdiction assignments
        // succeeds; the other cannot replace it, even after the lock is freed.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let tasks: Vec<_> = ["eu", "us"]
            .into_iter()
            .map(|jurisdiction| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    (
                        jurisdiction,
                        FileJournal::open_in_partition(
                            &path,
                            "w",
                            jurisdiction,
                            JournalAckMode::Sync,
                        )
                        .is_ok(),
                    )
                })
            })
            .collect();
        let results: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(_, opened)| *opened).count(), 1);
        for (jurisdiction, won) in results {
            assert_eq!(
                FileJournal::open_in_partition(&path, "w", jurisdiction, JournalAckMode::Sync)
                    .is_ok(),
                won
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn partition_fence_symlinks_are_not_authority() {
        // Oracle: a symlink cannot supply a trusted creation record or make a
        // scoped open create/mutate a journal through an alias.
        let dir = TempDir::new().unwrap();
        let outside = dir.path().join("outside");
        std::fs::write(
            &outside,
            br#"{"version":1,"world_id":"w","jurisdiction":"eu"}"#,
        )
        .unwrap();
        let path = dir.path().join("journal.jsonl");
        std::os::unix::fs::symlink(&outside, dir.path().join("journal.jsonl.partition.json"))
            .unwrap();
        assert!(FileJournal::open_in_partition(&path, "w", "eu", JournalAckMode::Sync).is_err());
        assert!(!path.exists());
        let alias = dir.path().join("alias.jsonl");
        std::os::unix::fs::symlink(&outside, &alias).unwrap();
        let before = std::fs::read(&outside).unwrap();
        assert!(FileJournal::open_in_partition(&alias, "w", "eu", JournalAckMode::Sync).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), before);

        let assigned = dir.path().join("assigned.jsonl");
        drop(FileJournal::open_in_partition(&assigned, "w", "eu", JournalAckMode::Sync).unwrap());
        OpenOptions::new()
            .append(true)
            .open(&assigned)
            .unwrap()
            .write_all(b"{torn")
            .unwrap();
        let before = std::fs::read(&assigned).unwrap();
        let unscoped_alias = dir.path().join("unscoped.jsonl");
        std::os::unix::fs::symlink(&assigned, &unscoped_alias).unwrap();
        assert!(FileJournal::open(&unscoped_alias, "w").is_err());
        let hard_alias = dir.path().join("hard.jsonl");
        std::fs::hard_link(&assigned, &hard_alias).unwrap();
        assert!(FileJournal::open(&hard_alias, "w").is_err());
        assert_eq!(std::fs::read(&assigned).unwrap(), before);
    }

    #[test]
    fn partition_fence_bare_relative_path() {
        // Oracle: relative and absolute journal paths have identical creation
        // semantics. A subprocess avoids changing the other tests' cwd.
        const CHILD: &str = "ID_PARTITION_RELATIVE_TEST";
        if std::env::var_os(CHILD).is_some() {
            drop(
                FileJournal::open_in_partition(
                    Path::new("journal.jsonl"),
                    "w",
                    "eu",
                    JournalAckMode::Sync,
                )
                .unwrap(),
            );
            FileJournal::open_in_partition(
                Path::new("journal.jsonl"),
                "w",
                "eu",
                JournalAckMode::Sync,
            )
            .unwrap();
            return;
        }
        let dir = TempDir::new().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "world_store::tests::partition_fence_bare_relative_path",
            ])
            .env(CHILD, "1")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(dir.path().join("journal.jsonl").exists());
    }

    proptest::proptest! {
        #[test]
        fn partition_fence_binding_is_immutable(home in "[a-z]{1,8}", foreign in "[a-z]{1,8}") {
            // Oracle: immutable binding invariant for every distinct label pair.
            proptest::prop_assume!(home != foreign);
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("journal.jsonl");
            drop(FileJournal::open_in_partition(&path, "w", &home, JournalAckMode::Sync).unwrap());
            let metadata = dir.path().join("journal.jsonl.partition.json");
            let before = std::fs::read(&metadata).unwrap();
            proptest::prop_assert!(FileJournal::open_in_partition(&path, "w", &foreign, JournalAckMode::Sync).is_err());
            proptest::prop_assert!(FileJournal::open(&path, "w").is_err());
            proptest::prop_assert_eq!(std::fs::read(&metadata).unwrap(), before);
            proptest::prop_assert!(FileJournal::open_in_partition(&path, "w", &home, JournalAckMode::Sync).is_ok());
        }
    }

    #[test]
    fn journal_rejects_a_second_live_handle_and_reopens_after_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (journal, _) = FileJournal::open(&path, "w").unwrap();

        let error = FileJournal::open(&path, "w")
            .expect_err("a second handle must not own the same journal");
        let message = format!("{error:#}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains("already open"), "{message}");

        drop(journal);
        FileJournal::open(&path, "w").expect("the lock is released when the handle drops");
    }

    // #8239 group commit: a batch lands every entry in order, readable
    // back exactly, after the one concatenated write + sync.
    #[test]
    fn journal_append_batch_lands_every_entry_in_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let batch: Vec<JournalEntry> = ["a", "b", "c"]
            .iter()
            .map(|name| JournalEntry::CapsGranted {
                granted: vec![(*name).to_owned()],
            })
            .collect();
        {
            let (mut journal, entries) = FileJournal::open(&path, "w").unwrap();
            assert_eq!(entries.len(), 1, "a fresh journal holds only its header");
            journal.append_batch(&batch).unwrap();
        }
        let entries = journal_entries(dir.path());
        assert_eq!(entries.len(), 1 + batch.len(), "{entries:?}");
        assert_eq!(&entries[1..], batch.as_slice());
    }

    // #8239: the byte cap is checked on the batch total before writing,
    // so a refused batch writes nothing - never a prefix of the batch.
    #[test]
    fn journal_append_batch_over_the_cap_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, _) = FileJournal::open_with_max_bytes(&path, "w", 512).unwrap();
        let batch: Vec<JournalEntry> = (0..8)
            .map(|n| JournalEntry::CapsGranted {
                granted: vec![format!("capability-number-{n}-with-a-long-name")],
            })
            .collect();
        let error = journal.append_batch(&batch).unwrap_err();
        assert!(error.to_string().contains("byte cap"), "{error:#}");
        drop(journal);
        let entries = journal_entries(dir.path());
        assert_eq!(
            entries.len(),
            1,
            "only the header survives a refused batch: {entries:?}"
        );
    }

    // #8239: a batch containing an entry that needs a header version bump
    // falls back to the per-entry path, whose bump semantics are unchanged.
    #[test]
    fn journal_append_batch_with_a_bump_entry_bumps_the_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let profile = crate::world_limits::WorldExecutionProfile {
            wasm_fuel: Some(7),
            ..Default::default()
        };
        let (mut journal, _) = FileJournal::open(&path, "w").unwrap();
        journal
            .append_batch(&[
                JournalEntry::CapsGranted {
                    granted: vec!["chat".to_owned()],
                },
                JournalEntry::ExecutionProfile { profile },
            ])
            .unwrap();
        drop(journal);
        let entries = journal_entries(dir.path());
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert!(matches!(
            entries[0],
            JournalEntry::Created {
                version: crate::world::JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        assert!(matches!(entries[2], JournalEntry::ExecutionProfile { .. }));
    }

    // #557 (ack modes): the default journal is Sync, and its on-disk
    // image is byte-for-byte the pre-mode contract — the header and
    // each entry as one JSON line, every append synced before it
    // returns (unsynced_appends never leaves zero).
    #[test]
    fn journal_default_ack_mode_is_sync_and_writes_the_exact_old_bytes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let entries: Vec<JournalEntry> = (0..3)
            .map(|n| JournalEntry::CapsGranted {
                granted: vec![format!("cap-{n}")],
            })
            .collect();
        let mut expected = journal_line(&JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        });
        {
            let (mut journal, _) = FileJournal::open(&path, "w").unwrap();
            assert_eq!(journal.ack_mode(), JournalAckMode::Sync);
            assert_eq!(
                <FileJournal as WorldJournal>::ack_mode(&journal),
                JournalAckMode::Sync
            );
            assert_eq!(journal.unsynced_appends(), 0, "the header append syncs");
            for entry in &entries {
                journal.append(entry).unwrap();
                assert_eq!(
                    journal.unsynced_appends(),
                    0,
                    "a sync-mode append is durable when it returns"
                );
                expected.extend_from_slice(&journal_line(entry));
            }
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            expected,
            "the default mode writes exactly the bytes the old code wrote"
        );
    }

    // #557 (ack modes): WorldLimits carries the mode and defaults to
    // Sync, so every existing world keeps the old contract.
    #[test]
    fn world_limits_default_ack_mode_is_sync() {
        assert_eq!(
            WorldLimits::default().journal_ack_mode,
            JournalAckMode::Sync
        );
    }

    // #557 (ack modes): Async acknowledges after the write and before
    // any sync — the journal's own accounting shows the appends
    // uncovered — and an explicit sync point flushes them; everything
    // written replays on reopen, in order.
    #[test]
    fn journal_async_mode_acks_before_sync_and_sync_point_flushes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let entries: Vec<JournalEntry> = (0..3)
            .map(|n| JournalEntry::CapsGranted {
                granted: vec![format!("cap-{n}")],
            })
            .collect();
        {
            let (mut journal, _) =
                FileJournal::open_with_ack_mode(&path, "w", JournalAckMode::Async).unwrap();
            assert_eq!(journal.ack_mode(), JournalAckMode::Async);
            assert_eq!(
                journal.unsynced_appends(),
                1,
                "even the header waits for a sync point in async mode"
            );
            for entry in &entries {
                journal.append(entry).unwrap();
            }
            assert_eq!(
                journal.unsynced_appends(),
                1 + entries.len() as u64,
                "no append synced on the ack path"
            );
            // The bytes are written (a reopen would read them); they
            // are simply not durable yet.
            assert_eq!(journal_entries(dir.path()).len(), 1 + entries.len());
            <FileJournal as WorldJournal>::sync(&mut journal).unwrap();
            assert_eq!(journal.unsynced_appends(), 0, "the sync point flushes");
        }
        let (_journal, replayed) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(&replayed[1..], entries.as_slice());
    }

    // #557 (ack modes): Batched syncs exactly when the pending count
    // reaches JOURNAL_ACK_BATCH_SIZE — never before — counted across
    // append and append_batch entries; the loss window is the partial
    // batch, and a sync point flushes it early.
    #[test]
    fn journal_batched_mode_syncs_once_per_batch() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let entry = JournalEntry::CapsGranted {
            granted: vec!["chat".to_owned()],
        };
        let (mut journal, _) =
            FileJournal::open_with_ack_mode(&path, "w", JournalAckMode::Batched).unwrap();
        assert_eq!(journal.unsynced_appends(), 1, "the header is pending");
        // Fill to one short of the bound: still no sync.
        for _ in 1..JOURNAL_ACK_BATCH_SIZE - 1 {
            journal.append(&entry).unwrap();
        }
        assert_eq!(journal.unsynced_appends(), JOURNAL_ACK_BATCH_SIZE - 1);
        // The append that fills the batch flushes it.
        journal.append(&entry).unwrap();
        assert_eq!(
            journal.unsynced_appends(),
            0,
            "the batch boundary syncs exactly once"
        );
        // A batch append counts its entries toward the same bound:
        // BATCH-1 entries pend without flushing, and one more single
        // append crosses the bound and flushes for all of them.
        let batch = vec![entry.clone(); (JOURNAL_ACK_BATCH_SIZE - 1) as usize];
        journal.append_batch(&batch).unwrap();
        assert_eq!(journal.unsynced_appends(), JOURNAL_ACK_BATCH_SIZE - 1);
        journal.append(&entry).unwrap();
        assert_eq!(journal.unsynced_appends(), 0);
        // A partial batch waits, and sync() flushes it early.
        journal.append(&entry).unwrap();
        assert_eq!(journal.unsynced_appends(), 1);
        <FileJournal as WorldJournal>::sync(&mut journal).unwrap();
        assert_eq!(journal.unsynced_appends(), 0);
        drop(journal);
        let (_journal, replayed) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(
            replayed.len() as u64,
            1 + 2 * JOURNAL_ACK_BATCH_SIZE,
            "every batched append replayed: {replayed:?}"
        );
    }

    // #557 (ack modes): the mode never reinterprets existing state —
    // a journal written under one mode replays identically under any
    // other, including async appends that never reached a sync point
    // (written bytes survive a process exit; only an OS crash could
    // have dropped them, which is the mode's documented window).
    #[test]
    fn journal_mode_switch_on_reopen_replays_identically() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let entries: Vec<JournalEntry> = (0..4)
            .map(|n| JournalEntry::CapsGranted {
                granted: vec![format!("cap-{n}")],
            })
            .collect();
        {
            let (mut journal, _) =
                FileJournal::open_with_ack_mode(&path, "w", JournalAckMode::Async).unwrap();
            for entry in &entries {
                journal.append(entry).unwrap();
            }
            // No sync point: drop is a process exit, not an OS crash.
        }
        {
            let (mut journal, replayed) = FileJournal::open(&path, "w").unwrap();
            assert_eq!(&replayed[1..], entries.as_slice());
            journal.append(&entries[0]).unwrap();
        }
        let (_journal, replayed) =
            FileJournal::open_with_ack_mode(&path, "w", JournalAckMode::Batched).unwrap();
        assert_eq!(replayed.len(), 1 + entries.len() + 1);
        assert_eq!(&replayed[1..=entries.len()], entries.as_slice());
    }

    // #557 (ack modes): a compaction is a sync point in every mode —
    // the rewrite is built, synced and renamed, so afterwards nothing
    // acknowledged is left uncovered.
    #[test]
    fn journal_async_mode_compaction_is_a_sync_point() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, mut entries) =
            FileJournal::open_with_ack_mode(&path, "w", JournalAckMode::Async).unwrap();
        let entry = JournalEntry::CapsGranted {
            granted: vec!["chat".to_owned()],
        };
        journal.append(&entry).unwrap();
        entries.push(entry);
        assert!(journal.unsynced_appends() > 0);
        journal.compact(&entries).unwrap();
        assert_eq!(journal.unsynced_appends(), 0);
        drop(journal);
        let (_journal, replayed) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(replayed, entries);
    }

    // #560 (option B): appends refuse at the byte cap, the file never
    // exceeds it, and the accepted prefix is exactly what a reopen reads.
    #[test]
    fn journal_appends_are_refused_at_the_byte_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, entries) = FileJournal::open_with_max_bytes(&path, "w", 512).unwrap();
        assert_eq!(entries.len(), 1, "a fresh journal holds only its header");
        let entry = JournalEntry::CapsGranted {
            granted: vec!["chat".to_owned()],
        };
        let mut appended = 0usize;
        let refusal = loop {
            match journal.append(&entry) {
                Ok(()) => appended += 1,
                Err(error) => break error,
            }
        };
        assert!(appended > 0, "some entries fit under the cap");
        assert!(
            refusal.to_string().contains("byte cap"),
            "the refusal names the cap: {refusal}"
        );
        let file_len = std::fs::metadata(&path).unwrap().len();
        assert!(
            file_len <= 512,
            "the file never exceeds the cap: {file_len}"
        );
        drop(journal);
        let (_journal, replayed) = FileJournal::open_with_max_bytes(&path, "w", 512).unwrap();
        assert_eq!(replayed.len(), 1 + appended);
    }

    // #10730: the transaction path is the same byte cap, refused BEFORE the
    // rewrite. A refusal writes nothing (the file is byte-identical), never a
    // prefix of the transaction, and the accepted prefix replays exactly.
    #[test]
    fn journal_transactions_are_refused_at_the_byte_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let cap = 512;
        let (mut journal, entries) = FileJournal::open_with_max_bytes(&path, "w", cap).unwrap();
        assert_eq!(entries.len(), 1, "a fresh journal holds only its header");
        let entry = JournalEntry::CapsGranted {
            granted: vec!["chat".to_owned()],
        };
        let mut appended = 0usize;
        while journal.append(&entry).is_ok() {
            appended += 1;
        }
        assert!(appended > 0, "some entries fit under the cap");
        let accepted = std::fs::read(&path).unwrap();
        assert!(
            accepted.len() as u64 <= cap,
            "the cap held before the transaction"
        );

        // One entry of the transaction fits; the other cannot, so the whole
        // transaction must be refused and nothing may land.
        let transaction = vec![
            entry.clone(),
            JournalEntry::CapsGranted {
                granted: vec!["x".repeat(cap as usize)],
            },
        ];
        let refusal = journal.append_transaction(&transaction).unwrap_err();
        assert!(
            refusal.to_string().contains("byte cap"),
            "the refusal names the cap: {refusal}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            accepted,
            "a refused transaction writes nothing, not even its fitting prefix"
        );
        assert!(
            std::fs::metadata(&path).unwrap().len() <= cap,
            "the file never exceeds the cap"
        );
        drop(journal);
        let (_journal, replayed) = FileJournal::open_with_max_bytes(&path, "w", cap).unwrap();
        assert_eq!(
            replayed.len(),
            1 + appended,
            "replay equals the accepted prefix"
        );

        // A transaction that fits still commits as one unit.
        let fitting_path = dir.path().join("fitting.jsonl");
        let (mut fitting, _entries) =
            FileJournal::open_with_max_bytes(&fitting_path, "w", cap).unwrap();
        let fitting_transaction = vec![entry.clone(), entry.clone()];
        fitting.append_transaction(&fitting_transaction).unwrap();
        drop(fitting);
        let (_journal, replayed) =
            FileJournal::open_with_max_bytes(&fitting_path, "w", cap).unwrap();
        assert_eq!(
            replayed.len(),
            1 + fitting_transaction.len(),
            "a fitting transaction commits atomically"
        );
    }

    // #10730 acceptance 2: `compact` stays the shrink path. An over-cap
    // journal still opens, still refuses growth through both append paths,
    // and a checkpoint-sized rewrite brings it back under the cap so writes
    // resume.
    #[test]
    fn journal_compact_trims_an_over_cap_journal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let cap = 512;
        let (mut journal, entries) = FileJournal::open_with_max_bytes(&path, "w", cap).unwrap();
        let bulk = JournalEntry::CapsGranted {
            granted: vec!["x".repeat(200)],
        };
        let mut over_cap = entries.clone();
        over_cap.extend((0..4).map(|_| bulk.clone()));
        // A rewrite is not capped: this is how a checkpoint shrinks a
        // journal, and how a journal can legitimately be over its cap.
        journal.compact(&over_cap).unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() > cap,
            "the rewrite produced an over-cap journal"
        );
        assert!(
            journal.append(&bulk).is_err(),
            "appends refuse while over the cap"
        );
        assert!(
            journal
                .append_transaction(std::slice::from_ref(&bulk))
                .is_err(),
            "transactions refuse while over the cap"
        );
        // The checkpoint trim (one snapshot entry) is the remedy.
        let trimmed = vec![
            entries[0].clone(),
            JournalEntry::CapsGranted {
                granted: vec!["checkpoint".to_owned()],
            },
        ];
        journal.compact(&trimmed).unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() <= cap,
            "the trim brings the journal back under the cap"
        );
        journal.append(&bulk).unwrap();
        drop(journal);
        let (_journal, replayed) = FileJournal::open_with_max_bytes(&path, "w", cap).unwrap();
        assert_eq!(replayed.len(), trimmed.len() + 1, "writes resumed");
    }

    // The production cap leaves ordinary appends untouched, and a capped
    // journal does not disturb a healthy neighbour (the DoS probe shape).
    #[test]
    fn journal_under_the_production_cap_appends_normally() {
        let dir = TempDir::new().unwrap();
        let entry = JournalEntry::CapsGranted {
            granted: vec!["chat".to_owned()],
        };
        let capped_path = dir.path().join("capped.jsonl");
        let (mut capped, _entries) =
            FileJournal::open_with_max_bytes(&capped_path, "w", 128).unwrap();
        let big_entry = JournalEntry::CapsGranted {
            granted: vec!["x".repeat(200)],
        };
        assert!(capped.append(&big_entry).is_err(), "a tiny cap refuses");
        let probe_path = dir.path().join("probe.jsonl");
        let (mut probe, _entries) = FileJournal::open(&probe_path, "probe").unwrap();
        probe.append(&entry).unwrap();
        assert!(MAX_JOURNAL_BYTES >= 64 * 1024 * 1024);
    }

    fn scan(data: Vec<u8>) -> (anyhow::Result<(Vec<JournalEntry>, u64, bool)>, usize) {
        let counting = CountingRead {
            inner: std::io::Cursor::new(data),
            served: 0,
        };
        let mut reader = BufReader::with_capacity(64, counting);
        let result = FileJournal::scan_entries(&mut reader, Path::new("journal.jsonl"));
        (result, reader.into_inner().served)
    }

    #[test]
    fn journal_scan_reports_complete_corruption_without_reading_the_remainder() {
        let good = journal_line(&JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        });
        let mut data = good.clone();
        data.extend_from_slice(b"{\"not\":\"an entry\"}\n");
        let tail_start = data.len();
        data.extend(std::iter::repeat_n(b'x', 1 << 20));
        let (result, served) = scan(data);
        let err = result
            .err()
            .expect("a malformed complete line is corruption, not a torn tail");
        assert!(
            err.to_string()
                .contains(&format!("corrupt at byte {}", good.len())),
            "diagnostic keeps the path/offset form: {err}"
        );
        assert!(
            served <= tail_start + 64,
            "scan served {served} bytes of a {}-byte journal: it drained the remainder",
            tail_start + (1 << 20)
        );
    }

    #[test]
    fn journal_scan_complete_corrupt_last_line_is_corruption_not_torn() {
        let good = journal_line(&JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        });
        let mut data = good.clone();
        data.extend_from_slice(b"garbage\n");
        let (result, _) = scan(data);
        let err = result.err().expect("complete corrupt line must fail");
        assert!(
            err.to_string()
                .contains(&format!("corrupt at byte {}", good.len())),
            "{err}"
        );
    }

    #[test]
    fn journal_scan_recovers_a_torn_unterminated_tail() {
        let good = journal_line(&JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        });
        let mut data = good.clone();
        data.extend_from_slice(b"{\"Created\":{\"wor");
        let (result, _) = scan(data);
        let (entries, good_len, torn) = result.expect("a torn tail recovers");
        assert_eq!(entries.len(), 1);
        assert_eq!(good_len, good.len() as u64);
        assert!(torn);
    }

    #[test]
    fn journal_scan_reads_a_clean_journal_to_eof() {
        let first = journal_line(&JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        });
        let mut data = first.clone();
        data.extend_from_slice(&first);
        let (result, _) = scan(data);
        let (entries, good_len, torn) = result.expect("clean journal scans");
        assert_eq!(entries.len(), 2);
        assert_eq!(good_len, (first.len() * 2) as u64);
        assert!(!torn);
    }

    fn service(admin: Option<&str>) -> impl FnOnce(WorldHandle) -> WorldService {
        let admin = admin.map(str::to_owned);
        move |handle| WorldService::new(handle, admin)
    }

    struct DenyAccountAndMembershipGate;

    impl BoundaryGate for DenyAccountAndMembershipGate {
        fn check(&self, point: EnforcementPoint, _: &str) -> FrameworkOutcome {
            match point {
                EnforcementPoint::AccountCreation
                | EnforcementPoint::WorldMembership
                | EnforcementPoint::BlobStorage => FrameworkOutcome::Deny {
                    placement: Vec::new(),
                    assurance: Vec::new(),
                },
                _ => FrameworkOutcome::Allow,
            }
        }
    }

    #[test]
    fn world_store_gate_installer_applies_account_and_world_membership_guards() {
        // Oracle: id-directory's BoundaryGate contract refuses signup for a
        // Deny outcome. The same configured gate must protect account signup
        // and account-bound world membership before the core is spawned.
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let (credential, _) = core.directory_mut().sign_up("ann").unwrap();
        install_world_boundary_gate(&mut core, Some(Arc::new(DenyAccountAndMembershipGate)));
        assert!(core.directory_mut().sign_up("bo").is_err());
        assert!(
            core.issue_for_account(
                &credential,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "ann",
            )
            .is_err()
        );
        assert!(core.check_blob_boundary_for(None).is_err());
    }

    #[tokio::test]
    async fn replication_feed_snapshots_exact_bytes_and_follows_durable_appends() {
        let dir = TempDir::new().unwrap();
        let (service, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        let world = service.world();
        let (world_id, first_source, mut first_updates) = world.replication_feed().await.unwrap();
        let (second_world_id, second_source, mut second_updates) =
            world.replication_feed().await.unwrap();
        let durable_bytes = std::fs::read(dir.path().join("journal.jsonl")).unwrap();
        assert_eq!(world_id, "lobby");
        assert_eq!(second_world_id, world_id);
        assert_eq!(first_source.header_version, second_source.header_version);
        assert_eq!(first_source.byte_len, durable_bytes.len() as u64);

        // The actor is free to append before the network materializes either
        // source. The captured length still bounds both snapshots to this cut.
        world
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        let first = first_source.read().unwrap();
        let second = second_source.read().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.bytes, durable_bytes);
        assert!(first.bytes.ends_with(b"\n"));
        for update in [
            first_updates.recv().await.unwrap(),
            second_updates.recv().await.unwrap(),
        ] {
            match update {
                crate::world::ReplicationUpdate::Append {
                    header_version,
                    entry,
                } => {
                    assert_eq!(header_version, first.header_version);
                    assert!(entry.ends_with(b"\n"));
                    assert!(matches!(
                        serde_json::from_slice::<JournalEntry>(&entry).unwrap(),
                        JournalEntry::Isolation {
                            isolation: Isolation::Unisolated
                        }
                    ));
                }
                other => panic!("expected durable append, got {other:?}"),
            }
        }
        world.shutdown().await.unwrap();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn verified_replica_opens_offline_and_matches_writer_records_without_mutating_journal() {
        use crate::world::{
            DurableReceiver, JOURNAL_VERSION, ReplicationEnvelope, SignedReplicationEnvelope,
            frame_digest,
        };
        use ed25519_dalek::SigningKey;
        use iroh_blobs::store::mem::MemStore;

        let writer_dir = TempDir::new().unwrap();
        let replica_dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let (writer, _) = open_world(
            writer_dir.path(),
            "replica-offline",
            WorldLimits::default(),
            RuntimeLimits::default(),
            move |handle| {
                WorldService::new(handle, Some("writer-admin".to_owned())).with_blob_store(blobs)
            },
        )
        .await
        .unwrap();
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        let hash = module_hash(&wasm);
        writer
            .install_wasm(Some("writer-admin"), wasm, 42, &hash)
            .await
            .unwrap();
        let (_, spectator) = writer
            .world()
            .issue("spectator", WorldScopes::GUEST)
            .await
            .unwrap();
        for _ in 0..3 {
            writer
                .world()
                .input(spectator.clone(), b"inc".to_vec())
                .await
                .unwrap();
        }
        let (writer_sequence, writer_records) = writer.world().records();
        assert_eq!(writer_sequence, 4);
        assert_eq!(
            writer_records
                .get("count")
                .and_then(serde_json::Value::as_i64),
            Some(3)
        );

        // Copy the active content-addressed module to the replica directory;
        // the journal receiver verifies only the journal bytes in this phase.
        let replica_modules = replica_dir.path().join("modules");
        std::fs::create_dir_all(&replica_modules).unwrap();
        std::fs::copy(
            writer_dir
                .path()
                .join("modules")
                .join(format!("{hash}.wasm")),
            replica_modules.join(format!("{hash}.wasm")),
        )
        .unwrap();
        let journal_bytes = std::fs::read(writer_dir.path().join("journal.jsonl")).unwrap();
        let receiver_key = SigningKey::from_bytes(&[81; 32]);
        let pinned_source = *receiver_key.verifying_key().as_bytes();
        let nonce = [19; crate::world::SESSION_NONCE_BYTES];
        // The snapshot must carry the writer journal's own header version:
        // issuing the spectator records its join time, which raises the
        // header to JOURNAL_VERSION_JOIN_TIMES (#11960).
        let header_version = match serde_json::from_slice::<JournalEntry>(
            journal_bytes.split(|byte| *byte == b'\n').next().unwrap(),
        )
        .unwrap()
        {
            JournalEntry::Created { version, .. } => version,
            other => panic!("writer journal must open with its header, got {other:?}"),
        };
        assert!(header_version >= JOURNAL_VERSION);
        let mut receiver =
            DurableReceiver::open(replica_dir.path(), "replica-offline", pinned_source).unwrap();
        receiver.begin_session(nonce);
        let snapshot = ReplicationEnvelope::snapshot(
            "replica-offline",
            pinned_source,
            header_version,
            nonce,
            [0; 32],
            journal_bytes.clone(),
        )
        .unwrap();
        let snapshot_cursor = snapshot.cursor_after;
        let snapshot_frame = SignedReplicationEnvelope::sign(snapshot, &receiver_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let previous = frame_digest(&snapshot_frame);
        receiver.accept_frame(&snapshot_frame).unwrap();
        let terminal = ReplicationEnvelope::terminal(
            "replica-offline",
            pinned_source,
            header_version,
            nonce,
            snapshot_cursor,
            previous,
        );
        let terminal_frame = SignedReplicationEnvelope::sign(terminal, &receiver_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        receiver.accept_frame(&terminal_frame).unwrap();
        drop(receiver);

        let expected_bytes = std::fs::read(replica_dir.path().join("journal.jsonl")).unwrap();
        assert_eq!(expected_bytes, journal_bytes);
        // The writer is unreachable before replica open.
        writer.world().shutdown().await.unwrap();
        let (replica, report, cursor) = open_replica(
            replica_dir.path(),
            "replica-offline",
            pinned_source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("must-be-stripped")),
        )
        .await
        .unwrap();
        assert!(replica.world().is_read_only());
        assert_eq!(report.program_error, None);
        assert_eq!(report.sequence, writer_sequence);
        assert_eq!(cursor.sequence, writer_sequence);
        assert_eq!(report.module_hash.as_deref(), Some(hash.as_str()));
        // PR #4708 Codex P2: the replica cleared its blob store, leaving
        // the spectator module download with no source. The restored
        // module is served read-only from the replica's module
        // directory, while uploads stay refused (the writer admin token
        // was stripped at replica open).
        let expected_wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        assert_eq!(replica.module_bytes(&hash).await.unwrap(), expected_wasm);
        assert!(
            replica
                .install_wasm(Some("must-be-stripped"), expected_wasm, 42, &hash)
                .await
                .is_err()
        );
        assert_eq!(report.replayed_inputs, 3);
        let (replica_sequence, replica_records) = replica.world().records();
        assert_eq!(replica_sequence, writer_sequence);
        assert_eq!(*replica_records, *writer_records);
        assert!(
            replica
                .world()
                .input(spectator, b"no".to_vec())
                .await
                .is_err()
        );
        replica.world().shutdown().await.unwrap();
        assert_eq!(
            std::fs::read(replica_dir.path().join("journal.jsonl")).unwrap(),
            expected_bytes,
            "serving never mutates the verified journal"
        );
    }

    #[tokio::test]
    async fn offline_replica_serves_friend_delegation_from_directory_facts_without_writer() {
        use crate::directory::Actor;
        use crate::world::{
            DurableReceiver, JournalEntry, MAX_JOURNAL_VERSION as JOURNAL_VERSION,
            ReplicationEnvelope, SignedReplicationEnvelope, WorldCore, frame_digest,
            journal_entry_line,
        };
        use ed25519_dalek::SigningKey;

        let world_id = "replica-directory-offline";
        let mut writer_core = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann_credential, _) = writer_core.directory_mut().sign_up("Ann").unwrap();
        let ann = writer_core
            .directory()
            .account_for(&ann_credential)
            .unwrap();
        let (bo_credential, _) = writer_core.directory_mut().sign_up("Bo").unwrap();
        let bo = writer_core.directory().account_for(&bo_credential).unwrap();
        writer_core
            .directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        writer_core
            .directory_mut()
            .request_friend(&ann_credential, &bo, world_id, 1)
            .unwrap();
        writer_core
            .directory_mut()
            .accept_friend(&bo_credential, &ann, world_id, 2)
            .unwrap();
        let (owner, owner_token) = writer_core
            .issue_for_account(
                &ann_credential,
                crate::world::WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (spectator, spectator_token) = writer_core
            .delegate_to_friend(
                &owner_token,
                &bo,
                crate::world::WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo spectator",
            )
            .unwrap();
        let subjects = std::collections::BTreeSet::from([ann, bo]);
        let directory_facts = writer_core
            .directory()
            .replica_facts_for(&subjects)
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            },
            writer_core.issued_entry(owner.id).unwrap(),
            writer_core.issued_entry(spectator.id).unwrap(),
        ];
        let journal_bytes: Vec<u8> = entries.iter().flat_map(journal_entry_line).collect();
        drop(writer_core);

        let replica_dir = TempDir::new().unwrap();
        let directory_path = replica_dir.path().join("directory.jsonl");
        let mut directory_bytes = Vec::new();
        for entry in &directory_facts {
            serde_json::to_writer(&mut directory_bytes, entry).unwrap();
            directory_bytes.push(b'\n');
        }
        // A crash-debris fragment is ignored in memory and is not repaired by
        // the replica's read-only open path.
        directory_bytes.extend_from_slice(b"{\"torn\":");
        std::fs::write(&directory_path, &directory_bytes).unwrap();

        let source_key = SigningKey::from_bytes(&[82; 32]);
        let source = *source_key.verifying_key().as_bytes();
        let nonce = [21; crate::world::SESSION_NONCE_BYTES];
        let mut receiver = DurableReceiver::open(replica_dir.path(), world_id, source).unwrap();
        receiver.begin_session(nonce);
        let snapshot = ReplicationEnvelope::snapshot(
            world_id,
            source,
            JOURNAL_VERSION,
            nonce,
            [0; 32],
            journal_bytes.clone(),
        )
        .unwrap();
        let cursor = snapshot.cursor_after;
        let snapshot_frame = SignedReplicationEnvelope::sign(snapshot, &source_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let previous = frame_digest(&snapshot_frame);
        receiver.accept_frame(&snapshot_frame).unwrap();
        let terminal = ReplicationEnvelope::terminal(
            world_id,
            source,
            JOURNAL_VERSION,
            nonce,
            cursor,
            previous,
        );
        let terminal_frame = SignedReplicationEnvelope::sign(terminal, &source_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        receiver.accept_frame(&terminal_frame).unwrap();
        drop(receiver);

        // No writer world/handle is alive or reachable after this point.
        let (replica, _, _) = open_replica(
            replica_dir.path(),
            world_id,
            source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("writer-admin")),
        )
        .await
        .unwrap();
        let snapshot = replica
            .world()
            .snapshot(spectator_token.clone())
            .await
            .unwrap();
        assert_eq!(snapshot.current_sequence, 0);
        assert!(
            replica
                .world()
                .chat(spectator_token, "read only")
                .await
                .is_err()
        );
        replica.world().shutdown().await.unwrap();
        assert_eq!(
            std::fs::read(&directory_path).unwrap(),
            directory_bytes,
            "read-only startup must not repair the directory sidecar"
        );
        assert_eq!(
            std::fs::read(replica_dir.path().join("journal.jsonl")).unwrap(),
            journal_bytes
        );
    }

    /// Fixture for the gated replica open (F12.13 phase 5): a durable
    /// replica directory whose journal carries an owner, Bo — present
    /// at the verified cursor via a journaled join — and Cy, who holds
    /// a journal-issued JOIN capability but was never present (the NEW
    /// spectator of the policy-B ruling). The replication source key
    /// doubles as the admission-lease issuer, as it does for a pinned
    /// writer.
    struct GatedReplicaFixture {
        dir: TempDir,
        source: [u8; 32],
        source_key: ed25519_dalek::SigningKey,
        present_token: crate::world::JoinCapability,
        present_id: u64,
        new_token: crate::world::JoinCapability,
        new_id: u64,
    }

    fn gated_replica_fixture(world_id: &str) -> GatedReplicaFixture {
        use ed25519_dalek::SigningKey;

        use crate::directory::Actor;
        use crate::world::{
            DurableReceiver, JournalEntry, MAX_JOURNAL_VERSION as JOURNAL_VERSION,
            ReplicationEnvelope, SignedReplicationEnvelope, WorldCore, WorldEvent, WorldEventKind,
            frame_digest, journal_entry_line,
        };

        let mut writer_core = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann_credential, _) = writer_core.directory_mut().sign_up("Ann").unwrap();
        let (bo_credential, _) = writer_core.directory_mut().sign_up("Bo").unwrap();
        let (cy_credential, _) = writer_core.directory_mut().sign_up("Cy").unwrap();
        let ann = writer_core
            .directory()
            .account_for(&ann_credential)
            .unwrap();
        let bo = writer_core.directory().account_for(&bo_credential).unwrap();
        let cy = writer_core.directory().account_for(&cy_credential).unwrap();
        for account in [&ann, &bo, &cy] {
            writer_core
                .directory_mut()
                .verify_account(Actor::Server, account)
                .unwrap();
        }
        let (owner, _owner_token) = writer_core
            .issue_for_account(
                &ann_credential,
                crate::world::WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (present, present_token) = writer_core
            .issue_for_account(
                &bo_credential,
                crate::world::WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo spectator",
            )
            .unwrap();
        let (newcomer, new_token) = writer_core
            .issue_for_account(
                &cy_credential,
                crate::world::WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Cy spectator",
            )
            .unwrap();
        let subjects = std::collections::BTreeSet::from([ann, bo, cy]);
        let directory_facts = writer_core
            .directory()
            .replica_facts_for(&subjects)
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            },
            writer_core.issued_entry(owner.id).unwrap(),
            writer_core.issued_entry(present.id).unwrap(),
            writer_core.issued_entry(newcomer.id).unwrap(),
            JournalEntry::System {
                event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: serde_json::json!({
                            "cap": "players",
                            "event": "joined",
                            "participant": {"id": present.id, "name": "Bo spectator"}
                        })
                        .to_string(),
                    },
                },
            },
        ];
        let journal_bytes: Vec<u8> = entries.iter().flat_map(journal_entry_line).collect();
        drop(writer_core);

        let dir = TempDir::new().unwrap();
        let mut directory_bytes = Vec::new();
        for entry in &directory_facts {
            serde_json::to_writer(&mut directory_bytes, entry).unwrap();
            directory_bytes.push(b'\n');
        }
        std::fs::write(dir.path().join("directory.jsonl"), &directory_bytes).unwrap();

        let source_key = SigningKey::from_bytes(&[82; 32]);
        let source = *source_key.verifying_key().as_bytes();
        let nonce = [23; crate::world::SESSION_NONCE_BYTES];
        let mut receiver = DurableReceiver::open(dir.path(), world_id, source).unwrap();
        receiver.begin_session(nonce);
        let snapshot = ReplicationEnvelope::snapshot(
            world_id,
            source,
            JOURNAL_VERSION,
            nonce,
            [0; 32],
            journal_bytes,
        )
        .unwrap();
        let cursor = snapshot.cursor_after;
        let snapshot_frame = SignedReplicationEnvelope::sign(snapshot, &source_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let previous = frame_digest(&snapshot_frame);
        receiver.accept_frame(&snapshot_frame).unwrap();
        let terminal = ReplicationEnvelope::terminal(
            world_id,
            source,
            JOURNAL_VERSION,
            nonce,
            cursor,
            previous,
        );
        let terminal_frame = SignedReplicationEnvelope::sign(terminal, &source_key)
            .unwrap()
            .encode_frame()
            .unwrap();
        receiver.accept_frame(&terminal_frame).unwrap();
        drop(receiver);
        GatedReplicaFixture {
            dir,
            source,
            source_key,
            present_token,
            present_id: present.id,
            new_token,
            new_id: newcomer.id,
        }
    }

    // Oracle: policy B ruling (#806 comment 6634842315) on the durable
    // open path — a valid writer-signed lease admits the new spectator,
    // and the status reports source, world, cursor and the remaining
    // admission window.
    #[tokio::test]
    async fn gated_replica_open_with_a_valid_lease_admits_and_reports() {
        use crate::world::SignedAdmissionLease;

        let world_id = "replica-gated-valid";
        let fixture = gated_replica_fixture(world_id);
        let now = crate::world::unix_ms();
        let lease =
            SignedAdmissionLease::issue(world_id, &fixture.source_key, now, 3_600_000).unwrap();
        let (replica, _report, cursor, status) = open_replica_with_admission(
            fixture.dir.path(),
            world_id,
            fixture.source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("writer-admin")),
            Some(lease),
            now,
        )
        .await
        .unwrap();
        assert_eq!(status.world_id, world_id);
        assert_eq!(status.source, fixture.source);
        assert_eq!(status.cursor, cursor);
        assert_eq!(status.cursor.entries, 5);
        assert_eq!(status.lease_expires_at_unix_ms, Some(now + 3_600_000));
        let remaining = status.lease_remaining_ms.unwrap();
        assert!(remaining > 0 && remaining <= 3_600_000, "{remaining}");
        assert!(status.admits_new_spectators);
        assert_eq!(
            replica
                .world()
                .participant_id(fixture.present_token.clone())
                .await
                .unwrap(),
            fixture.present_id
        );
        assert_eq!(
            replica
                .world()
                .participant_id(fixture.new_token.clone())
                .await
                .unwrap(),
            fixture.new_id
        );
        replica.world().shutdown().await.unwrap();
    }

    // Oracle: policy B — an expired lease does NOT fail the durable
    // open: reads keep serving, the participant present at the cursor
    // is unaffected, and only new admissions stop.
    #[tokio::test]
    async fn gated_replica_open_with_an_expired_lease_serves_but_admits_no_one_new() {
        use crate::world::SignedAdmissionLease;

        let world_id = "replica-gated-expired";
        let fixture = gated_replica_fixture(world_id);
        let now = crate::world::unix_ms();
        let lease =
            SignedAdmissionLease::issue(world_id, &fixture.source_key, now - 7_200_000, 3_600_000)
                .unwrap();
        let (replica, _report, _cursor, status) = open_replica_with_admission(
            fixture.dir.path(),
            world_id,
            fixture.source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("writer-admin")),
            Some(lease),
            now,
        )
        .await
        .unwrap();
        assert_eq!(status.lease_remaining_ms, Some(0));
        assert!(!status.admits_new_spectators);
        assert!(
            replica
                .world()
                .snapshot(fixture.present_token.clone())
                .await
                .is_ok()
        );
        assert!(
            replica
                .world()
                .participant_id(fixture.new_token.clone())
                .await
                .is_err(),
            "expired lease must refuse the new spectator"
        );
        replica.world().shutdown().await.unwrap();
    }

    // Oracle: policy B — gated open holding no lease serves existing
    // participants and admits no new spectator.
    #[tokio::test]
    async fn gated_replica_open_without_a_lease_admits_no_one_new() {
        let world_id = "replica-gated-no-lease";
        let fixture = gated_replica_fixture(world_id);
        let now = crate::world::unix_ms();
        let (replica, _report, _cursor, status) = open_replica_with_admission(
            fixture.dir.path(),
            world_id,
            fixture.source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("writer-admin")),
            None,
            now,
        )
        .await
        .unwrap();
        assert_eq!(status.lease_expires_at_unix_ms, None);
        assert_eq!(status.lease_remaining_ms, None);
        assert!(!status.admits_new_spectators);
        assert!(
            replica
                .world()
                .snapshot(fixture.present_token.clone())
                .await
                .is_ok()
        );
        assert!(
            replica
                .world()
                .participant_id(fixture.new_token.clone())
                .await
                .is_err()
        );
        replica.world().shutdown().await.unwrap();
    }

    // A lease signed by anyone other than the pinned source is an
    // operator misconfiguration: the open fails before serving.
    #[tokio::test]
    async fn gated_replica_open_refuses_a_lease_from_another_source() {
        use crate::world::SignedAdmissionLease;

        let world_id = "replica-gated-wrong-source";
        let fixture = gated_replica_fixture(world_id);
        let now = crate::world::unix_ms();
        let foreign_key = ed25519_dalek::SigningKey::from_bytes(&[99; 32]);
        let lease = SignedAdmissionLease::issue(world_id, &foreign_key, now, 3_600_000).unwrap();
        let error = open_replica_with_admission(
            fixture.dir.path(),
            world_id,
            fixture.source,
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(Some("writer-admin")),
            Some(lease),
            now,
        )
        .await
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("pinned source"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn write_lease_entries_survive_jsonl_reopen_with_epoch_intact() {
        // Design #2819 phase 2 (#3804): lease entries appended to a
        // fresh (version 1) journal bump its header to version 16 on
        // first write (#13711: stamped positions), and a reopen +
        // replay rebuilds the same lease: holder, monotonic epoch,
        // term, and transition history.
        use crate::world::{
            JOURNAL_VERSION_LEASE_POSITION, JournalEntry as Entry, WorldCore, WorldJournal,
            WorldLimits,
        };
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, _) = FileJournal::open(&path, "w").unwrap();
        journal
            .append(&Entry::LeaseHandedOff {
                from: "do".to_owned(),
                to: "native-1".to_owned(),
                epoch: 1,
                expires_at_ms: Some(30_000),
                position: 1,
            })
            .unwrap();
        journal
            .append(&Entry::LeaseRenewed {
                holder: "native-1".to_owned(),
                epoch: 1,
                expires_at_ms: 40_000,
                position: 2,
            })
            .unwrap();
        drop(journal);

        let (_journal, entries) = FileJournal::open(&path, "w").unwrap();
        assert!(matches!(
            entries[0],
            Entry::Created {
                version: JOURNAL_VERSION_LEASE_POSITION,
                ..
            }
        ));
        let restored = WorldCore::restore("w", WorldLimits::default(), entries).unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.holder.as_deref(), Some("native-1"));
        assert_eq!(lease.epoch, 1);
        assert_eq!(lease.expires_at_ms, Some(40_000));
        assert_eq!(lease.transitions().count(), 2);
    }

    #[test]
    fn replication_snapshot_repairs_an_unterminated_torn_tail() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (journal, _) = FileJournal::open(&path, "lobby").unwrap();
        let expected = std::fs::read(&path).unwrap();
        drop(journal);

        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{"torn":"entry""#)
            .unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), expected);

        let (mut journal, _) = FileJournal::open(&path, "lobby").unwrap();
        let snapshot = journal.replication_snapshot().unwrap();
        assert_eq!(snapshot.bytes, expected);
        assert_eq!(std::fs::read(&path).unwrap(), expected);
    }

    #[test]
    fn replication_snapshot_source_is_pinned_and_reads_only_its_captured_length() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, entries) = FileJournal::open(&path, "lobby").unwrap();
        journal
            .append(&JournalEntry::Isolation {
                isolation: Isolation::Unisolated,
            })
            .unwrap();
        let expected = std::fs::read(&path).unwrap();
        let source = journal.replication_snapshot_source().unwrap();
        assert_eq!(source.byte_len, expected.len() as u64);

        // Replacing the path after capture must not redirect the reader to a
        // compacted image; the source owns the old inode and captured length.
        journal.compact(&entries).unwrap();
        let snapshot = source.read().unwrap();
        assert_eq!(snapshot.bytes, expected);
        assert!(snapshot.bytes.ends_with(b"\n"));
        assert_ne!(std::fs::read(&path).unwrap(), snapshot.bytes);
    }

    #[tokio::test]
    async fn isolation_survives_a_restart() {
        let dir = TempDir::new().unwrap();
        let open = || {
            open_world(
                dir.path(),
                "lobby",
                WorldLimits::default(),
                RuntimeLimits::default(),
                service(None),
            )
        };
        let (svc, _) = open().await.unwrap();
        assert_eq!(svc.world().isolation().await.unwrap(), Isolation::Isolated);
        svc.world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        svc.world().shutdown().await.unwrap();

        let (svc, _) = open().await.unwrap();
        assert_eq!(
            svc.world().isolation().await.unwrap(),
            Isolation::Unisolated
        );
    }

    #[tokio::test]
    async fn sharing_settings_survive_a_restart() {
        let dir = TempDir::new().unwrap();
        let open = || {
            open_world(
                dir.path(),
                "lobby",
                WorldLimits::default(),
                RuntimeLimits::default(),
                service(None),
            )
        };
        let (svc, _) = open().await.unwrap();
        svc.world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        svc.world()
            .set_cross_world_write(Authority::Admin, true)
            .await
            .unwrap();
        svc.world()
            .force_cross_world_write(Authority::Admin)
            .await
            .unwrap();
        svc.world()
            .set_file_scope(Authority::Admin, FileScope::Unrestricted, Some(true))
            .await
            .unwrap();
        let before = svc.world().policy().await.unwrap();
        assert!(before.cross_world_write && before.cross_world_forced);
        assert!(before.file_write_override);
        svc.world().shutdown().await.unwrap();

        let (svc, _) = open().await.unwrap();
        assert_eq!(svc.world().policy().await.unwrap(), before);
    }

    #[tokio::test]
    async fn execution_profile_survives_restart_and_current_server_limits_win() {
        let dir = TempDir::new().unwrap();
        let mut tighter_server = RuntimeLimits::default();
        tighter_server.native.cpu_seconds = 2;
        let open = |runtime| {
            open_world(
                dir.path(),
                "profiled",
                WorldLimits::default(),
                runtime,
                move |handle| {
                    WorldService::new(handle, Some("admin".to_owned())).with_runtime_limits(runtime)
                },
            )
        };
        let (service, _) = open(RuntimeLimits::default()).await.unwrap();
        let requested = crate::world_limits::WorldExecutionProfile {
            native_cpu_seconds: Some(100),
            ..Default::default()
        };
        service
            .set_execution_profile(Some("admin"), requested)
            .await
            .unwrap();
        service.world().shutdown().await.unwrap();

        let (service, _) = open(tighter_server).await.unwrap();
        assert_eq!(
            service.world().execution_profile().await.unwrap(),
            requested
        );
        assert_eq!(
            requested
                .resolve(tighter_server)
                .unwrap()
                .native
                .cpu_seconds,
            2,
            "a persisted request must not raise the new server's hard ceiling"
        );
    }

    #[tokio::test]
    async fn presentation_policy_is_admin_only_and_survives_restart() {
        // #14731 slice 2b: the policy is journaled before the reply, the
        // journal header is raised to the version that names the entry,
        // and a reopened world reads the same policy.
        let dir = TempDir::new().unwrap();
        let open = || {
            open_world(
                dir.path(),
                "presented",
                WorldLimits::default(),
                RuntimeLimits::default(),
                |handle| WorldService::new(handle, Some("admin".to_owned())),
            )
        };
        let policy = id_core::presentation_policy::PresentationPolicy::from_json(
            r#"{"filters":{"disguise":"allow"},"shaders":{"enabled":true}}"#,
        )
        .unwrap();
        let (service, _) = open().await.unwrap();
        let refused = service
            .set_presentation_policy(Some("wrong"), policy.clone())
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("admin denied"), "{refused:#}");
        assert_eq!(
            service.presentation_policy().await.unwrap(),
            id_core::presentation_policy::PresentationPolicy::default()
        );
        assert_eq!(
            service
                .set_presentation_policy(Some("admin"), policy.clone())
                .await
                .unwrap(),
            policy
        );
        service.world().shutdown().await.unwrap();

        let (service, _) = open().await.unwrap();
        assert_eq!(service.presentation_policy().await.unwrap(), policy);
        service.world().shutdown().await.unwrap();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_lower_server_ceiling_refuses_a_restored_grant_and_the_default_keeps_it() {
        // Lounge subscribes to `time.tick` in every `wants`, so the restored
        // instance decides that grant on the real restore path.
        async fn open_lounge(dir: &Path, limits: WorldLimits) -> WorldService {
            let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
            let (service, _) = open_world(
                dir,
                "ceilinged",
                limits,
                RuntimeLimits::default(),
                move |handle| {
                    WorldService::new(handle, Some("admin".to_owned())).with_blob_store(blobs)
                },
            )
            .await
            .unwrap();
            service
        }
        let wasm = include_bytes!("../../../examples/lounge/lounge.wasm").to_vec();
        let hash = module_hash(&wasm);
        let dir = TempDir::new().unwrap();

        let service = open_lounge(dir.path(), WorldLimits::default()).await;
        service
            .install_wasm(Some("admin"), wasm, 1, &hash)
            .await
            .unwrap();
        service
            .world()
            .update_caps(vec!["time.tick".to_owned()], Vec::new())
            .await
            .unwrap();
        service.world().shutdown().await.unwrap();

        let without_tick: std::collections::BTreeSet<String> = crate::world_caps::catalog_names()
            .into_iter()
            .filter(|name| name != "time.tick")
            .collect();
        let lower = WorldLimits {
            server_caps: without_tick,
            ..WorldLimits::default()
        };
        let service = open_lounge(dir.path(), lower).await;
        let report = service.world().caps_report().await.unwrap();
        assert!(
            report.granted.contains(&"time.tick".to_owned()),
            "the grant is kept"
        );
        assert!(!report.effective.contains(&"time.tick".to_owned()));
        assert!(
            report
                .usage
                .get("time.tick")
                .is_some_and(|usage| usage.denied > 0),
            "{report:?}"
        );
        service.world().shutdown().await.unwrap();

        let service = open_lounge(dir.path(), WorldLimits::default()).await;
        let report = service.world().caps_report().await.unwrap();
        assert!(report.effective.contains(&"time.tick".to_owned()));
        assert!(
            report
                .usage
                .get("time.tick")
                .is_some_and(|usage| usage.allowed > 0 && usage.denied == 0),
            "{report:?}"
        );
        service.world().shutdown().await.unwrap();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn restored_program_uses_the_current_server_runtime_ceiling() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        let hash = module_hash(&wasm);
        let dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let (service, _) = open_world(
            dir.path(),
            "profiled-restore",
            WorldLimits::default(),
            RuntimeLimits::default(),
            move |handle| {
                WorldService::new(handle, Some("admin".to_owned()))
                    .with_blob_store(blobs)
                    .with_runtime_limits(RuntimeLimits::default())
            },
        )
        .await
        .unwrap();

        service
            .set_execution_profile(
                Some("admin"),
                crate::world_limits::WorldExecutionProfile {
                    wasm_fuel: Some(u64::MAX),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .install_wasm(Some("admin"), wasm, 1, &hash)
            .await
            .unwrap();
        service.world().shutdown().await.unwrap();

        let constrained_server = RuntimeLimits {
            sandbox: crate::sandbox::SandboxLimits {
                fuel: 1,
                ..crate::sandbox::SandboxLimits::default()
            },
            ..RuntimeLimits::default()
        };
        let fresh_blobs: iroh_blobs::api::Store = MemStore::new().into();
        let (restored, report) = open_world(
            dir.path(),
            "profiled-restore",
            WorldLimits::default(),
            constrained_server,
            move |handle| {
                WorldService::new(handle, Some("admin".to_owned()))
                    .with_blob_store(fresh_blobs)
                    .with_runtime_limits(constrained_server)
            },
        )
        .await
        .unwrap();

        assert_eq!(report.module_hash.as_deref(), Some(hash.as_str()));
        assert!(
            report.program_error.is_some(),
            "restoration should use the server's stricter fuel ceiling"
        );
        assert_eq!(restored.active_module_hash().await, None);
    }

    #[tokio::test]
    async fn chat_and_capabilities_survive_a_restart() {
        let dir = TempDir::new().unwrap();
        let (svc, report) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        assert_eq!(report.sequence, 0);
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        let (bob_p, bob) = svc.world().issue("bob", WorldScopes::GUEST).await.unwrap();
        svc.world().chat(ann.clone(), "first").await.unwrap();
        svc.world().chat(bob.clone(), "second").await.unwrap();
        assert!(svc.world().revoke(bob_p.id).await.unwrap());
        svc.world().shutdown().await.unwrap();

        let (svc, report) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        assert_eq!(report.sequence, 2);
        let snapshot = svc.world().snapshot(ann.clone()).await.unwrap();
        let text = serde_json::to_string(&snapshot.events).unwrap();
        assert!(text.contains("first") && text.contains("second"));
        assert!(
            svc.world().snapshot(bob).await.is_err(),
            "revocation is durable"
        );
        // New participants never reuse an old ID.
        let (carl, _) = svc.world().issue("carl", WorldScopes::GUEST).await.unwrap();
        assert_eq!(carl.id, 3);
        let event = svc.world().chat(ann, "third").await.unwrap();
        assert_eq!(event.sequence, 3);
    }

    #[test]
    fn journal_never_contains_capability_secrets() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (mut journal, _) = FileJournal::open(&path, "w").unwrap();
        let mut core = WorldCore::new("w", WorldLimits::default()).unwrap();
        let (participant, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        journal
            .append(&JournalEntry::Issued {
                issued_at: None,
                participant,
                digest: "00".repeat(32),
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            })
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let secret = token.expose().split_once('.').unwrap().1;
        assert!(!text.contains(secret));
    }

    #[tokio::test]
    async fn a_revoked_delegation_subtree_stays_revoked_after_restart() {
        let dir = TempDir::new().unwrap();
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        let (owner, owner_token) = svc.world().issue("ann", WorldScopes::ALL).await.unwrap();
        let (_, child_token) = svc
            .world()
            .attenuate(
                owner_token.clone(),
                WorldScopes::JOIN.union(WorldScopes::DELEGATE),
                CapabilityBounds::default(),
                "bo",
            )
            .await
            .unwrap();
        let (_, grandchild_token) = svc
            .world()
            .attenuate(
                child_token.clone(),
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "cy",
            )
            .await
            .unwrap();
        let (_, bob_token) = svc.world().issue("dee", WorldScopes::GUEST).await.unwrap();
        assert!(svc.world().revoke(owner.id).await.unwrap());
        svc.world().shutdown().await.unwrap();

        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        for token in [owner_token, child_token, grandchild_token] {
            assert!(svc.world().snapshot(token).await.is_err());
        }
        assert!(svc.world().snapshot(bob_token).await.is_ok());
    }

    #[tokio::test]
    async fn attenuated_capability_secrets_never_reach_the_journal() {
        let dir = TempDir::new().unwrap();
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        let (owner, owner_token) = svc.world().issue("ann", WorldScopes::ALL).await.unwrap();
        let (_, child_token) = svc
            .world()
            .attenuate(
                owner_token.clone(),
                WorldScopes::JOIN.union(WorldScopes::DELEGATE),
                CapabilityBounds::default(),
                "bo",
            )
            .await
            .unwrap();
        assert!(svc.world().revoke(owner.id).await.unwrap());
        svc.world().shutdown().await.unwrap();

        let journal = std::fs::read_to_string(dir.path().join("journal.jsonl")).unwrap();
        assert!(journal.contains("revoked"), "revocation is journaled");
        for token in [&owner_token, &child_token] {
            let secret = token.expose().split_once('.').unwrap().1;
            assert!(
                !journal.contains(secret),
                "a capability secret was journaled"
            );
        }
    }

    #[test]
    fn torn_final_entry_is_dropped_but_mid_file_corruption_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open(&path, "w").unwrap());
        let good = std::fs::read(&path).unwrap();

        let mut torn = good.clone();
        torn.extend_from_slice(b"{\"entry\":\"revoked\",\"partic");
        std::fs::write(&path, &torn).unwrap();
        let (_, entries) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), good, "torn tail truncated");

        let mut corrupt = good.clone();
        corrupt.extend_from_slice(b"garbage\n");
        corrupt.extend_from_slice(&good);
        std::fs::write(&path, &corrupt).unwrap();
        let err = FileJournal::open(&path, "w").unwrap_err();
        assert!(err.to_string().contains("corrupt"), "{err:#}");
    }

    #[test]
    fn blank_lines_before_between_and_after_entries_are_skipped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open(&path, "w").unwrap());
        let created = std::fs::read(&path).unwrap();
        let revoked = serde_json::to_vec(&JournalEntry::Revoked {
            revoked_at: None,
            participant_id: 1,
            by: None,
        })
        .unwrap();

        let mut with_blanks = b"\n".to_vec();
        with_blanks.extend_from_slice(&created);
        with_blanks.extend_from_slice(b"\n");
        with_blanks.extend_from_slice(&revoked);
        with_blanks.extend_from_slice(b"\n\n");
        std::fs::write(&path, &with_blanks).unwrap();

        let (_, entries) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(std::fs::read(&path).unwrap(), with_blanks);
    }

    #[test]
    fn a_terminated_entry_this_binary_cannot_read_is_refused_not_dropped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        drop(FileJournal::open(&path, "w").unwrap());
        let good = std::fs::read(&path).unwrap();

        // An entry from a newer binary, fully written, as the final line.
        let mut newer = good.clone();
        newer.extend_from_slice(b"{\"entry\":\"future_variant\",\"x\":1}\n");
        std::fs::write(&path, &newer).unwrap();
        let err = FileJournal::open(&path, "w").unwrap_err();
        assert!(err.to_string().contains("corrupt"), "{err:#}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            newer,
            "the unreadable entry is left in place, not truncated"
        );
    }

    #[tokio::test]
    async fn a_journal_for_another_world_is_refused() {
        let dir = TempDir::new().unwrap();
        // Capture the first world's handle so it can be shut down
        // explicitly: dropping the service alone leaves its actor -
        // and the actor's journal ownership (#8459) - alive, so the
        // second open would be refused as a concurrent open instead
        // of reaching the belongs-to check below.
        let first_handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured = first_handle.clone();
        drop(
            open_world(
                dir.path(),
                "one",
                WorldLimits::default(),
                RuntimeLimits::default(),
                move |handle| {
                    *captured.lock().unwrap() = Some(handle.clone());
                    service(None)(handle)
                },
            )
            .await
            .unwrap(),
        );
        first_handle
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        let err = open_world(
            dir.path(),
            "two",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("belongs to world"), "{err:#}");
    }

    #[test]
    fn module_dir_rejects_tampered_files_and_bad_hashes() {
        let dir = TempDir::new().unwrap();
        let modules = ModuleDir::new(dir.path().join("modules"));
        let hash = modules.save(b"\0asm\x01\0\0\0").unwrap();
        assert_eq!(modules.load(&hash).unwrap(), b"\0asm\x01\0\0\0");
        std::fs::write(
            dir.path().join("modules").join(format!("{hash}.wasm")),
            b"x",
        )
        .unwrap();
        assert!(modules.load(&hash).is_err());
        assert!(modules.load("../../etc/passwd").is_err());
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn roc_program_state_is_rebuilt_by_replay() {
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        let dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let make = |blobs: iroh_blobs::api::Store| {
            move |handle| WorldService::new(handle, Some("admin".to_owned())).with_blob_store(blobs)
        };
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            make(blobs.clone()),
        )
        .await
        .unwrap();
        let hash = module_hash(&wasm);
        svc.install_wasm(Some("admin"), wasm, 42, &hash)
            .await
            .unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        for _ in 0..3 {
            svc.world()
                .input(ann.clone(), b"inc".to_vec())
                .await
                .unwrap();
        }
        svc.world().chat(ann.clone(), "hi").await.unwrap();
        assert!(view(&svc, &ann).await.contains("count=3"));
        svc.world().shutdown().await.unwrap();

        let fresh_blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let (svc, report) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            make(fresh_blobs),
        )
        .await
        .unwrap();
        assert_eq!(report.program_error, None);
        assert_eq!(report.replayed_inputs, 3);
        assert_eq!(report.module_hash.as_deref(), Some(hash.as_str()));
        assert!(
            view(&svc, &ann).await.contains("count=3"),
            "state rebuilt by replay"
        );
        assert_eq!(
            svc.active_module_hash().await.as_deref(),
            Some(hash.as_str()),
            "the restored module is downloadable again"
        );
        svc.world()
            .input(ann.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert!(view(&svc, &ann).await.contains("count=4"));
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_native_world_installs_and_plays_in_process() {
        let wasm = include_bytes!("../../../examples/roc-counter/counter-native").to_vec();
        let dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let make = |blobs: iroh_blobs::api::Store| {
            move |handle| {
                WorldService::new(handle, Some("admin".to_owned()))
                    .with_blob_store(blobs)
                    .with_native_enabled()
            }
        };
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            make(blobs.clone()),
        )
        .await
        .unwrap();
        let hash = module_hash(&wasm);
        svc.install_wasm(Some("admin"), wasm, 7, &hash)
            .await
            .unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        // The actor reads records and wants right after install, like it
        // does in production.
        let records = svc.world().records().1;
        assert_eq!(
            records.get("count").and_then(serde_json::Value::as_i64),
            Some(0)
        );
        let _wants = svc.world().caps_report().await.unwrap();
        svc.world()
            .input(ann.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert!(view(&svc, &ann).await.contains("count=1"));
        svc.world().shutdown().await.unwrap();
    }

    fn journal_entries(dir: &Path) -> Vec<JournalEntry> {
        std::fs::read_to_string(dir.join("journal.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn the_journal_is_trimmed_behind_a_checkpoint_and_restart_resumes_from_it() {
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        let dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let limits = WorldLimits {
            checkpoint_every: 5,
            ..WorldLimits::default()
        };
        let make = |blobs: iroh_blobs::api::Store| {
            move |handle| WorldService::new(handle, Some("admin".to_owned())).with_blob_store(blobs)
        };
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            limits.clone(),
            RuntimeLimits::default(),
            make(blobs.clone()),
        )
        .await
        .unwrap();
        let hash = module_hash(&wasm);
        svc.install_wasm(Some("admin"), wasm, 42, &hash)
            .await
            .unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        let (bob_participant, bob) = svc.world().issue("bob", WorldScopes::GUEST).await.unwrap();
        for _ in 0..13 {
            svc.world()
                .input(ann.clone(), b"inc".to_vec())
                .await
                .unwrap();
        }
        svc.world().chat(ann.clone(), "hi").await.unwrap();
        assert!(svc.world().revoke(bob_participant.id).await.unwrap());
        assert!(view(&svc, &ann).await.contains("count=13"));
        let sequence = svc
            .world()
            .snapshot(ann.clone())
            .await
            .unwrap()
            .current_sequence;
        svc.world().shutdown().await.unwrap();

        // 13 inputs + install + chat, yet the journal is a handful of lines.
        let entries = journal_entries(dir.path());
        let checkpoints = entries
            .iter()
            .filter(|entry| matches!(entry, JournalEntry::Checkpoint { .. }))
            .count();
        let committed = entries
            .iter()
            .filter(|entry| matches!(entry, JournalEntry::Committed { .. }))
            .count();
        assert_eq!(checkpoints, 1, "{entries:?}");
        assert!(
            committed < 5,
            "history behind the checkpoint is gone: {entries:?}"
        );
        assert!(matches!(entries[0], JournalEntry::Created { .. }));
        assert!(
            !dir.path().join("journal.jsonl.tmp").exists(),
            "no staging file is left behind"
        );

        let fresh: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let (svc, report) = open_world(
            dir.path(),
            "lobby",
            limits,
            RuntimeLimits::default(),
            make(fresh),
        )
        .await
        .unwrap();
        assert_eq!(report.program_error, None);
        assert_eq!(report.sequence, sequence, "sequence continues");
        assert!(
            report.replayed_inputs < 5,
            "replay starts at the checkpoint"
        );
        assert!(
            view(&svc, &ann).await.contains("count=13"),
            "state from checkpoint + tail"
        );
        assert_eq!(
            svc.active_module_hash().await.as_deref(),
            Some(hash.as_str())
        );
        assert!(
            svc.world().view(bob).await.is_err(),
            "a revocation survives compaction"
        );
        svc.world()
            .input(ann.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert!(view(&svc, &ann).await.contains("count=14"));
        let page = svc
            .world()
            .events_after(ann.clone(), sequence)
            .await
            .unwrap();
        assert_eq!(page.events.len(), 1, "catch-up works across the checkpoint");
    }

    #[tokio::test]
    async fn a_program_that_cannot_snapshot_keeps_its_full_journal() {
        struct Plain;
        impl WorldProgram for Plain {
            fn update(&mut self, _: &crate::world::WorldEvent) -> Result<()> {
                Ok(())
            }
        }
        let dir = TempDir::new().unwrap();
        let limits = WorldLimits {
            checkpoint_every: 2,
            ..WorldLimits::default()
        };
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            limits,
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        svc.world()
            .install_program("hash".to_owned(), 1, Box::new(Plain))
            .await
            .unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        for _ in 0..6 {
            svc.world().input(ann.clone(), b"x".to_vec()).await.unwrap();
        }
        svc.world().shutdown().await.unwrap();
        let entries = journal_entries(dir.path());
        assert!(
            !entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::Checkpoint { .. })),
            "nothing is trimmed without a verified snapshot"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::Committed { .. }))
                .count(),
            7
        );
    }

    #[tokio::test]
    async fn a_failing_snapshot_leaves_the_journal_and_the_world_untouched() {
        struct Flaky;
        impl WorldProgram for Flaky {
            fn snapshot(&mut self) -> Result<Option<String>> {
                bail!("snapshot does not round-trip")
            }
        }
        let dir = TempDir::new().unwrap();
        let limits = WorldLimits {
            checkpoint_every: 2,
            ..WorldLimits::default()
        };
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            limits,
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        svc.world()
            .install_program("hash".to_owned(), 1, Box::new(Flaky))
            .await
            .unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        for _ in 0..5 {
            svc.world()
                .input(ann.clone(), b"x".to_vec())
                .await
                .expect("the world keeps accepting inputs");
        }
        svc.world().shutdown().await.unwrap();
        assert!(
            !journal_entries(dir.path())
                .iter()
                .any(|entry| matches!(entry, JournalEntry::Checkpoint { .. }))
        );
    }

    #[test]
    fn a_checkpoint_after_events_is_refused_on_restore() {
        let mut entries = vec![JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        }];
        entries.push(JournalEntry::Committed {
            event: crate::world::WorldEvent {
                sequence: 1,
                participant_id: 0,
                kind: crate::world::WorldEventKind::ProgramInstalled {
                    module_hash: "h".to_owned(),
                    seed: 1,
                    provenance: None,
                    trust: None,
                },
            },
        });
        entries.push(JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: Vec::new(),
            admin_history: None,
            open_presence: None,
        });
        let error = WorldCore::restore("w", WorldLimits::default(), entries).unwrap_err();
        assert!(error.to_string().contains("checkpoint"), "{error:#}");
    }

    #[test]
    fn schedule_entries_survive_the_file_journal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let written = [
            JournalEntry::ScheduleSet {
                id: "a".to_owned(),
                at_ms: 9,
                interval_ms: None,
                remaining: None,
                target: None,
            },
            JournalEntry::ScheduleCancelled { id: "a".to_owned() },
            JournalEntry::ScheduleSet {
                id: "b".to_owned(),
                at_ms: 10,
                interval_ms: None,
                remaining: None,
                target: None,
            },
            JournalEntry::ScheduleFired { id: "b".to_owned() },
        ];
        {
            // Opening a new journal writes its header first.
            let (mut journal, header) = FileJournal::open(&path, "w").unwrap();
            assert_eq!(header.len(), 1);
            for entry in &written {
                journal.append(entry).unwrap();
            }
        }
        let (_, reread) = FileJournal::open(&path, "w").unwrap();
        assert_eq!(reread[1..], written);
    }

    #[tokio::test]
    async fn durable_world_metrics_count_journal_appends() {
        // F11.7 slice 1: on a journaled world, the actor's metrics count
        // every durable append (fsync included) — the journal half of the
        // registry the session test cannot reach with an in-memory world.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let (journal, entries) = FileJournal::open(&path, "lobby").unwrap();
        let core = WorldCore::restore("lobby", WorldLimits::default(), entries)
            .unwrap()
            .core;
        let world = WorldHandle::spawn_durable(
            core,
            Box::<crate::world::EmptyWorldProgram>::default(),
            Some(Box::new(journal)),
        );
        world
            .set_isolation(
                crate::world::Authority::Admin,
                crate::world::Isolation::Unisolated,
            )
            .await
            .unwrap();
        let metrics = world.metrics().await.unwrap();
        assert!(
            metrics.journal_appends_total >= 1,
            "the isolation change was journaled: {metrics:?}"
        );
        assert_eq!(
            metrics.journal_append_latency.count, metrics.journal_appends_total,
            "every append lands in the latency histogram: {metrics:?}"
        );
        assert!(metrics.commands_total >= 1, "{metrics:?}");
        assert_eq!(metrics.command_latency.count, metrics.commands_total);
        world.shutdown().await.unwrap();
    }

    #[test]
    fn first_execution_profile_append_bumps_the_journal_header() {
        // Cross-version fixture for the issue-#227 contract: a version-1
        // journal stays version 1 until the first entry a version-1 binary
        // cannot read, then its header is bumped before that entry lands.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let profile = crate::world_limits::WorldExecutionProfile {
            wasm_fuel: Some(7),
            ..Default::default()
        };
        {
            let (mut journal, entries) = FileJournal::open(&path, "w").unwrap();
            assert!(matches!(
                entries[0],
                JournalEntry::Created {
                    version: JOURNAL_VERSION,
                    ..
                }
            ));
            journal
                .append(&JournalEntry::ScheduleSet {
                    id: "a".to_owned(),
                    at_ms: 9,
                    interval_ms: None,
                    remaining: None,
                    target: None,
                })
                .unwrap();
            // A v1-readable entry does not bump the header.
            let on_disk = journal_entries(dir.path());
            assert!(matches!(
                on_disk[0],
                JournalEntry::Created {
                    version: JOURNAL_VERSION,
                    ..
                }
            ));
            journal
                .append(&JournalEntry::ExecutionProfile { profile })
                .unwrap();
        }
        let entries = journal_entries(dir.path());
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert!(
            matches!(
                entries[0],
                JournalEntry::Created {
                    version: crate::world::JOURNAL_VERSION_EXECUTION_PROFILE,
                    ..
                }
            ),
            "header bumped before the profile entry: {entries:?}"
        );
        assert!(matches!(entries[1], JournalEntry::ScheduleSet { .. }));
        assert!(matches!(entries[2], JournalEntry::ExecutionProfile { .. }));
        assert!(
            !dir.path().join("journal.jsonl.tmp").exists(),
            "the atomic bump leaves no staging file behind"
        );
        // The new binary reads the bumped journal back, profile included.
        let restored = WorldCore::restore("w", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.execution_profile(), profile);
    }

    #[test]
    fn a_legacy_v1_journal_holding_a_profile_entry_is_bumped_on_open() {
        // A journal written before the issue-#227 contract: profile entry
        // already sitting under a version-1 header. Opening it with the new
        // binary bumps the header, so a rollback refuses it early.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let legacy = [
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::ExecutionProfile {
                profile: crate::world_limits::WorldExecutionProfile {
                    wasm_fuel: Some(3),
                    ..Default::default()
                },
            },
        ];
        let text: String = legacy
            .iter()
            .map(|entry| format!("{}\n", serde_json::to_string(entry).unwrap()))
            .collect();
        std::fs::write(&path, text).unwrap();

        let (_, entries) = FileJournal::open(&path, "w").unwrap();
        assert!(matches!(
            entries[0],
            JournalEntry::Created {
                version: crate::world::JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        let on_disk = journal_entries(dir.path());
        assert!(matches!(
            on_disk[0],
            JournalEntry::Created {
                version: crate::world::JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        assert_eq!(
            on_disk.len(),
            2,
            "no entry is lost in the bump: {on_disk:?}"
        );
    }

    #[test]
    fn a_legacy_v1_journal_holding_builder_audit_is_bumped_on_open() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let legacy = [
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::BuilderInstalled {
                by: 7,
                module_hash: "module".to_owned(),
                seed: 9,
            },
        ];
        let text: String = legacy
            .iter()
            .map(|entry| format!("{}\n", serde_json::to_string(entry).unwrap()))
            .collect();
        std::fs::write(&path, text).unwrap();

        let (_, entries) = FileJournal::open(&path, "w").unwrap();
        assert!(matches!(
            entries[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        assert!(matches!(
            entries[1],
            JournalEntry::BuilderInstalled { by: 7, .. }
        ));
        assert_eq!(journal_entries(dir.path()), entries);
    }

    #[test]
    fn install_event_and_builder_audit_are_one_journal_transaction() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let transaction = [
            JournalEntry::Committed {
                event: crate::world::WorldEvent {
                    sequence: 1,
                    participant_id: 0,
                    kind: WorldEventKind::ProgramInstalled {
                        module_hash: "module".to_owned(),
                        seed: 9,
                        provenance: None,
                        trust: None,
                    },
                },
            },
            JournalEntry::BuilderInstalled {
                by: 7,
                module_hash: "module".to_owned(),
                seed: 9,
            },
        ];
        let (mut journal, _) = FileJournal::open(&path, "w").unwrap();

        // A staging-file failure happens before replacement, leaving the
        // original complete journal and neither half of the transaction.
        let staging_dir = path.with_extension("jsonl.tmp");
        std::fs::create_dir(&staging_dir).unwrap();
        assert!(journal.append_transaction(&transaction).is_err());
        std::fs::remove_dir(&staging_dir).unwrap();
        let unchanged = journal_entries(dir.path());
        assert_eq!(unchanged.len(), 1, "failed transaction was not appended");
        WorldCore::restore("w", WorldLimits::default(), unchanged).unwrap();

        journal.append_transaction(&transaction).unwrap();
        drop(journal);
        let committed = journal_entries(dir.path());
        assert!(matches!(
            committed[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        assert!(matches!(committed[1], JournalEntry::Committed { .. }));
        assert!(matches!(
            committed[2],
            JournalEntry::BuilderInstalled { by: 7, .. }
        ));
        WorldCore::restore("w", WorldLimits::default(), committed).unwrap();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_missing_module_leaves_chat_working_and_the_program_unavailable() {
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
        let dir = TempDir::new().unwrap();
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            {
                let blobs = blobs.clone();
                move |handle| WorldService::new(handle, Some("a".to_owned())).with_blob_store(blobs)
            },
        )
        .await
        .unwrap();
        let hash = module_hash(&wasm);
        svc.install_wasm(Some("a"), wasm, 1, &hash).await.unwrap();
        let (_, ann) = svc.world().issue("ann", WorldScopes::GUEST).await.unwrap();
        svc.world().shutdown().await.unwrap();
        std::fs::remove_dir_all(dir.path().join("modules")).unwrap();

        let (svc, report) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            {
                move |handle| WorldService::new(handle, Some("a".to_owned())).with_blob_store(blobs)
            },
        )
        .await
        .unwrap();
        assert!(report.program_error.is_some());
        assert!(svc.world().view(ann.clone()).await.is_err());
        assert!(
            svc.world()
                .input(ann.clone(), b"inc".to_vec())
                .await
                .is_err()
        );
        svc.world().chat(ann, "still here").await.unwrap();
    }

    #[cfg(feature = "sandbox")]
    async fn view(svc: &WorldService, token: &crate::world::JoinCapability) -> String {
        svc.world().view(token.clone()).await.unwrap().unwrap()
    }

    #[test]
    fn storage_failure_makes_the_world_read_only() {
        struct Broken;
        impl WorldJournal for Broken {
            fn append(&mut self, _: &JournalEntry) -> Result<()> {
                bail!("disk full")
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let core = WorldCore::new("w", WorldLimits::default()).unwrap();
            let handle =
                WorldHandle::spawn_durable(core, Box::new(EmptyProgram), Some(Box::new(Broken)));
            let err = handle.issue("ann", WorldScopes::GUEST).await.unwrap_err();
            assert!(format!("{err:#}").contains("read-only"), "{err:#}");
            assert!(handle.issue("bob", WorldScopes::GUEST).await.is_err());
        });
    }

    fn remote_request(to: &str) -> crate::directory_view::DirectoryAction {
        crate::directory_view::DirectoryAction::RequestFriend {
            to: to.to_owned(),
            remote: Some(crate::directory_view::RemoteFriend {
                node: iroh::SecretKey::from_bytes(&[7; 32]).public().to_string(),
                audience: "lobby".to_owned(),
            }),
        }
    }

    async fn signed_up(svc: &WorldService, name: &str) -> crate::directory_auth::Caller {
        let secret = svc
            .directory(
                crate::directory_auth::Caller::default(),
                crate::directory_view::DirectoryAction::SignUp {
                    name: name.to_owned(),
                },
            )
            .await
            .unwrap()
            .credential
            .unwrap();
        crate::directory_auth::Caller::from_secret(&secret)
    }

    #[tokio::test]
    async fn a_remote_friend_request_whose_envelope_cannot_be_queued_changes_nothing() {
        let dir = TempDir::new().unwrap();
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        let ann = signed_up(&svc, "Ann").await;
        let outbox_path = dir.path().join("outbox.jsonl");
        let _ = std::fs::remove_file(&outbox_path);
        std::fs::create_dir_all(&outbox_path).unwrap();

        let bo = "ab".repeat(32);
        assert!(
            svc.directory(ann.clone(), remote_request(&bo))
                .await
                .is_err()
        );
        let view = svc
            .directory(ann, crate::directory_view::DirectoryAction::View)
            .await
            .unwrap()
            .view;
        let me = view.accounts.iter().find(|a| a.name == "Ann").unwrap();
        assert_eq!(
            me.outgoing.as_deref().unwrap_or_default(),
            [] as [String; 0]
        );
    }

    #[tokio::test]
    async fn a_remote_friend_request_is_queued_once_and_a_refused_repeat_queues_nothing() {
        let dir = TempDir::new().unwrap();
        let (svc, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            service(None),
        )
        .await
        .unwrap();
        let ann = signed_up(&svc, "Ann").await;
        let bo = "ab".repeat(32);

        svc.directory(ann.clone(), remote_request(&bo))
            .await
            .unwrap();
        assert!(
            svc.directory(ann.clone(), remote_request(&bo))
                .await
                .is_err()
        );
        let view = svc
            .directory(ann, crate::directory_view::DirectoryAction::View)
            .await
            .unwrap()
            .view;
        let me = view.accounts.iter().find(|a| a.name == "Ann").unwrap();
        assert_eq!(me.outgoing, Some(vec![bo]));
        assert_eq!(view.deliveries.len(), 1);
    }
}
