//! Durable worlds: an append-only journal plus content-addressed modules.
//!
//! A world directory holds:
//!
//! ```text
//! <dir>/journal.jsonl            one JournalEntry per line, fsynced per append
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
    JOURNAL_VERSION, JournalEntry, WorldCore, WorldEventKind, WorldHandle, WorldJournal,
    WorldLimits, WorldProgram, journal_version_required_by,
};
use crate::world_session::{WorldService, module_hash};

/// Append-only, fsynced JSON-lines journal.
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("open world journal {}", path.display()))?;

        let mut entries = Vec::new();
        let mut good_len: u64 = 0;
        let mut torn = false;
        {
            let mut reader = BufReader::new(&file);
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
                        let mut rest = Vec::new();
                        reader.read_to_end(&mut rest)?;
                        if complete || !rest.is_empty() {
                            bail!(
                                "world journal {} is corrupt at byte {good_len}: {}",
                                path.display(),
                                parsed.err().map_or_else(
                                    || "unterminated entry".to_owned(),
                                    |e| e.to_string()
                                )
                            );
                        }
                        torn = true;
                        break;
                    }
                }
            }
        }
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
}

impl WorldJournal for FileJournal {
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
        self.file.write_all(&line).context("write world journal")?;
        self.file.sync_data().context("sync world journal")?;
        Ok(())
    }

    fn compact(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
        let tmp = self.path.with_extension("jsonl.tmp");
        // Build the replacement beside the journal; the live file is untouched
        // until the rename, so any failure before it leaves the old journal.
        let built = (|| -> Result<File> {
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
            Ok(file)
        })();
        let file = match built {
            Ok(file) => file,
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
    let journal_path = dir.join("journal.jsonl");
    let id = world_id.to_owned();
    let (journal, entries) =
        tokio::task::spawn_blocking(move || FileJournal::open(&journal_path, &id))
            .await
            .context("journal open task")??;
    let journal_entries = entries.len();
    let mut restored = WorldCore::restore(world_id, limits, entries)?;
    let directory_path = dir.join("directory.jsonl");
    let directory =
        tokio::task::spawn_blocking(move || crate::directory::Directory::open(&directory_path))
            .await
            .context("directory open task")??;
    *restored.core.directory_mut() = directory;
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
    let handle = WorldHandle::spawn_durable(restored.core, program, Some(Box::new(journal)));
    let service = service(handle).with_module_dir(modules).with_outbox(outbox);
    if let Some(wasm) = restored_module {
        service.adopt_module(wasm).await?;
    }
    Ok((service, report))
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

#[derive(Debug, Default)]
struct EmptyProgram;
impl WorldProgram for EmptyProgram {}

/// Restore a program of either tier. A native module (an ELF worker) is
/// spawned from its stored file; a Wasm module runs in the sandbox. Both
/// start from the checkpoint snapshot when the journal has one.
async fn restore_program(
    modules: &ModuleDir,
    spec: &crate::world::RestoredProgram,
    replay: &[crate::world::WorldEvent],
    runtime: crate::world_limits::RuntimeLimits,
) -> Result<(Box<dyn WorldProgram>, Vec<u8>)> {
    let modules = modules.clone();
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
    use tempfile::TempDir;

    use super::*;
    use crate::world::{Authority, CapabilityBounds, FileScope, Isolation, WorldScopes};
    use crate::world_limits::RuntimeLimits;

    fn service(admin: Option<&str>) -> impl FnOnce(WorldHandle) -> WorldService {
        let admin = admin.map(str::to_owned);
        move |handle| WorldService::new(handle, admin)
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
        let wasm = include_bytes!("../examples/lounge/lounge.wasm").to_vec();
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

        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
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
        drop(
            open_world(
                dir.path(),
                "one",
                WorldLimits::default(),
                RuntimeLimits::default(),
                service(None),
            )
            .await
            .unwrap(),
        );
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
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
        let wasm = include_bytes!("../examples/roc-counter/counter-native").to_vec();
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
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
            transfers: None,
            snapshot: "s".to_owned(),
            events: Vec::new(),
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
            },
            JournalEntry::ScheduleCancelled { id: "a".to_owned() },
            JournalEntry::ScheduleSet {
                id: "b".to_owned(),
                at_ms: 10,
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
                    version: JOURNAL_VERSION_EXECUTION_PROFILE,
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
                version: JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        let on_disk = journal_entries(dir.path());
        assert!(matches!(
            on_disk[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_EXECUTION_PROFILE,
                ..
            }
        ));
        assert_eq!(
            on_disk.len(),
            2,
            "no entry is lost in the bump: {on_disk:?}"
        );
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_missing_module_leaves_chat_working_and_the_program_unavailable() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
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
        assert!(me.outgoing.as_deref().unwrap_or_default().is_empty());
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
