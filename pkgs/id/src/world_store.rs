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
    JOURNAL_VERSION, JournalEntry, WorldCore, WorldHandle, WorldJournal, WorldLimits, WorldProgram,
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
                match serde_json::from_slice::<JournalEntry>(&line) {
                    Ok(entry) if complete => {
                        entries.push(entry);
                        good_len += read as u64;
                    }
                    parsed => {
                        // Only an unterminated or unparsable *last* line is a
                        // torn append; anything after it means corruption.
                        let mut rest = Vec::new();
                        reader.read_to_end(&mut rest)?;
                        if !rest.is_empty() {
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

        let mut journal = Self {
            file,
            path: path.to_owned(),
            broken: false,
        };
        if entries.is_empty() {
            let header = JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            };
            journal.append(&header)?;
            entries.push(header);
        }
        Ok((journal, entries))
    }
}

impl WorldJournal for FileJournal {
    fn append(&mut self, entry: &JournalEntry) -> Result<()> {
        ensure!(
            !self.broken,
            "world journal is unusable after a failed compaction"
        );
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
        Some(spec) => match restore_program(&modules, spec, &restored.replay, runtime).await {
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
    use crate::world::{Authority, CapabilityBounds, Isolation, WorldScopes};
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
                CapabilityBounds {
                    expires_at: None,
                    uses: Some(3),
                },
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
        assert_eq!(view(&svc, &ann).await, "count=3");
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
        assert_eq!(view(&svc, &ann).await, "count=3", "state rebuilt by replay");
        assert_eq!(
            svc.active_module_hash().await.as_deref(),
            Some(hash.as_str()),
            "the restored module is downloadable again"
        );
        svc.world()
            .input(ann.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert_eq!(view(&svc, &ann).await, "count=4");
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
        assert_eq!(view(&svc, &ann).await, "count=1");
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
            limits,
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
        assert_eq!(view(&svc, &ann).await, "count=13");
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
        assert_eq!(
            view(&svc, &ann).await,
            "count=13",
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
        assert_eq!(view(&svc, &ann).await, "count=14");
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
                },
            },
        });
        entries.push(JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            snapshot: "s".to_owned(),
            events: Vec::new(),
        });
        let error = WorldCore::restore("w", WorldLimits::default(), entries).unwrap_err();
        assert!(error.to_string().contains("checkpoint"), "{error:#}");
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
}
