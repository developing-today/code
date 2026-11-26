//! Host mode for the `id` binary: a supervisor over one data root of
//! world directories (#10693, packet P1 of the hosted-worlds v1 design
//! `doc/2026-11-16_design_id6283_hosted_worlds_v1.md` §§3–5).
//!
//! A world is its directory (`journal.jsonl` + `modules/`, the same local
//! layout a laptop host uses); world internals — [`crate::world_store`]
//! replay, ceilings, programs — are untouched. This module adds the only
//! new long-lived piece the design permits: the supervisor's own state.
//!
//! - [`LifecycleState`]: exactly three operator-visible states — live,
//!   asleep, archived — with transitions as explicit operations.
//! - [`HostFile`]: the small host file beside the data root recording
//!   each world's lifecycle state, the data-root path, and the
//!   state-change history (time, actor, prior state). It is rebuildable
//!   from the directories it describes; journals remain the only
//!   authority on world contents.
//! - [`Supervisor`]: the in-memory supervisor. Opening a data root loads
//!   (or rebuilds) the host file and replays every world recorded live;
//!   `start` is journal replay then hold, `stop` is drain (actor
//!   shutdown) then record.
//! - [`Census`]: per-state counts + bytes for the operator surface
//!   (packet P4), following the #1067 accounting shape (counts and
//!   bytes per state; tier mapping belongs to P4, not here).
//!
//! Serve-attach (clients reaching started worlds over the network) and
//! the `id host` CLI surface are follow-up slices, not this module: the
//! state machine records `live` only after a successful replay, so a
//! later slice can attach transport without changing these transitions.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::host_snapshots::{
    SNAPSHOT_DIR_NAME, SnapshotPolicy, SnapshotRun, SnapshotSet, is_due, list_sets, prune_plan,
    remove_sets, set_file_name, usage, world_snapshot_dir,
};
use crate::world::{Authority, WorldHandle, WorldLimits};
use crate::world_doctor::{JournalFacts, journal_facts};
use crate::world_export::{
    ExportManifest, export_world_to_file, import_world_from_file, read_export_manifest,
};
use crate::world_limits::RuntimeLimits;
use crate::world_session::WorldService;
use crate::world_storage::{JOURNAL_FILE, WalkLimits, inventory};
use crate::world_store::{OpenReport, open_world};

/// Version of the [`HostFile`] schema written by this module.
pub const HOST_FILE_VERSION: u32 = 1;

/// Name of the host file inside a data root.
pub const HOST_FILE_NAME: &str = "host.json";

/// Exactly three operator-visible lifecycle states (F1 of the v1 design).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleState {
    /// World process running (replayed, held by the supervisor).
    Live,
    /// No process; journal + modules at rest. The safe default.
    #[default]
    Asleep,
    /// At rest; no automatic wake — explicit operator restore first.
    Archived,
}

impl LifecycleState {
    /// All states, for exhaustive transition tests.
    pub const ALL: [LifecycleState; 3] = [
        LifecycleState::Live,
        LifecycleState::Asleep,
        LifecycleState::Archived,
    ];
}

impl std::fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecycleState::Live => write!(f, "live"),
            LifecycleState::Asleep => write!(f, "asleep"),
            LifecycleState::Archived => write!(f, "archived"),
        }
    }
}

/// How an asleep world comes back live (design §10.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeMode {
    /// A client's connection wakes it ([`WAKE_ON_CONNECT`]); membership
    /// is still checked by the world before it serves. The default.
    #[default]
    Connect,
    /// Wake-on-connect is off: only an operator `start` brings it live.
    Operator,
}

impl WakeMode {
    fn is_default(&self) -> bool {
        *self == WakeMode::Connect
    }
}

impl std::fmt::Display for WakeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WakeMode::Connect => write!(f, "connect"),
            WakeMode::Operator => write!(f, "operator"),
        }
    }
}

impl std::str::FromStr for WakeMode {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "connect" => Ok(WakeMode::Connect),
            "operator" => Ok(WakeMode::Operator),
            other => bail!("wake mode {other:?} is not `connect` or `operator`"),
        }
    }
}

/// One recorded lifecycle transition: when, by whom, from what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateChange {
    /// Unix seconds (UTC) when the transition was recorded.
    pub at_secs: u64,
    /// Operator identity, or `wake-on-connect` for demand wake.
    pub actor: String,
    /// State before the transition.
    pub prior: LifecycleState,
}

/// One world's supervisor-side record. The journal stays the only
/// authority on world *contents*; this is authority on lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldRecord {
    /// Current lifecycle state.
    pub state: LifecycleState,
    /// Newest-last transition history.
    #[serde(default)]
    pub history: Vec<StateChange>,
    /// Operator pin: the world is never closed for idleness or to make
    /// room for another world (design §10.2). It still stops when an
    /// operator stops it or the host shuts down.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// How the world wakes from asleep.
    #[serde(default, skip_serializing_if = "WakeMode::is_default")]
    pub wake: WakeMode,
    /// What the host found the last time it replayed this world while
    /// opening the data root, after a restart or a crash (#10696).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_recovery: Option<RecoveryRecord>,
    /// This world's own snapshot cadence and retention (#13739), in place
    /// of the host-wide default. An off policy here turns scheduled
    /// snapshots off for this world alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshots: Option<SnapshotPolicy>,
}

/// The result of replaying a world the host file recorded live, when a
/// host opened the data root: the world was live when the last host
/// stopped, by shutdown or by crash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    /// Unix seconds (UTC) when the replay finished.
    pub at_secs: u64,
    /// The journal ended in an incomplete line before the replay: the
    /// last append was cut off, and only the complete entries replayed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub torn_tail: bool,
    /// Last committed sequence after the replay.
    pub sequence: u64,
    /// Journal entries after the replay.
    pub journal_entries: u64,
    /// Inputs replayed into the restored program.
    pub replayed_inputs: u64,
    /// Why the program could not be restored, when the world opened
    /// without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_error: Option<String>,
    /// Why the replay failed, when it did; the world stays recorded live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RecoveryRecord {
    fn replayed(at_secs: u64, torn_tail: bool, report: &OpenReport) -> Self {
        Self {
            at_secs,
            torn_tail,
            sequence: report.sequence,
            journal_entries: u64::try_from(report.journal_entries).unwrap_or(u64::MAX),
            replayed_inputs: u64::try_from(report.replayed_inputs).unwrap_or(u64::MAX),
            program_error: report.program_error.clone(),
            error: None,
        }
    }

    fn failed(at_secs: u64, torn_tail: bool, error: String) -> Self {
        Self {
            at_secs,
            torn_tail,
            error: Some(error),
            ..Self::default()
        }
    }
}

/// The host file beside the data root: which worlds exist, their
/// lifecycle states, the data-root path, and state-change history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFile {
    /// Schema version ([`HOST_FILE_VERSION`]).
    pub version: u32,
    /// Canonical data-root path this file describes. A file that names a
    /// different root is refused: copy the directories and rebuild
    /// instead of editing this by hand.
    pub data_root: PathBuf,
    /// World name (directory name) → record.
    #[serde(default)]
    pub worlds: BTreeMap<String, WorldRecord>,
    /// Host-wide snapshot cadence and retention for live worlds (#13739).
    /// Absent means scheduled snapshots are off unless a world sets its
    /// own policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshots: Option<SnapshotPolicy>,
    /// Where snapshot sets live, when not `<data root>/.snapshots`: an
    /// absolute path, typically another disk or a mounted off-host store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_dir: Option<PathBuf>,
}

impl HostFile {
    /// Empty host file for `root`.
    pub fn empty(root: &Path) -> Self {
        Self {
            version: HOST_FILE_VERSION,
            data_root: root.to_path_buf(),
            worlds: BTreeMap::new(),
            snapshots: None,
            snapshot_dir: None,
        }
    }

    /// The directory that holds snapshot sets for the data root `root`.
    #[must_use]
    pub fn snapshot_root(&self, root: &Path) -> PathBuf {
        self.snapshot_dir
            .clone()
            .unwrap_or_else(|| root.join(SNAPSHOT_DIR_NAME))
    }

    /// The snapshot policy `name` follows: its own, else the host-wide
    /// default, else off.
    #[must_use]
    pub fn snapshot_policy(&self, name: &str) -> SnapshotPolicy {
        self.worlds
            .get(name)
            .and_then(|record| record.snapshots)
            .or(self.snapshots)
            .unwrap_or(SnapshotPolicy::OFF)
    }

    /// Set the host-wide snapshot policy (`world` is `None`) or one
    /// world's own; `None` as the policy clears it, so a world follows the
    /// host default again.
    ///
    /// # Errors
    ///
    /// Fails for a policy a host cannot follow and a world this file does
    /// not record.
    pub fn set_snapshot_policy(
        &mut self,
        world: Option<&str>,
        policy: Option<SnapshotPolicy>,
    ) -> Result<()> {
        if let Some(policy) = &policy {
            policy.validate()?;
        }
        match world {
            Some(name) => self.record_mut(name)?.snapshots = policy,
            None => self.snapshots = policy,
        }
        Ok(())
    }

    /// Store snapshot sets in `dir` instead of `<data root>/.snapshots`;
    /// `None` goes back to the default. Sets already taken stay where they
    /// are.
    ///
    /// # Errors
    ///
    /// Refuses a relative path.
    pub fn set_snapshot_dir(&mut self, dir: Option<PathBuf>) -> Result<()> {
        if let Some(dir) = &dir {
            if !dir.is_absolute() {
                bail!(
                    "the snapshot directory must be an absolute path, not {}",
                    dir.display()
                );
            }
        }
        self.snapshot_dir = dir;
        Ok(())
    }

    /// Path of the host file inside `root`.
    pub fn path_for(root: &Path) -> PathBuf {
        root.join(HOST_FILE_NAME)
    }

    /// The host file for `root` as it stands, writing nothing: the saved
    /// file when there is one, otherwise one rebuilt in memory from the
    /// directories (every journal-bearing directory, asleep). For
    /// read-only operator views such as `id host list`, which must never
    /// create or change the host file.
    ///
    /// # Errors
    ///
    /// As [`HostFile::load`] when the file exists, otherwise as
    /// [`HostFile::rebuild`].
    pub fn read_or_rebuild(root: &Path) -> Result<Self> {
        if Self::path_for(root).is_file() {
            Self::load(root)
        } else {
            Self::rebuild(root)
        }
    }

    /// Load and validate the host file for `root`.
    ///
    /// # Errors
    ///
    /// Fails when the file is missing, unreadable, the wrong version, or
    /// names a different data root. Callers that want to (re)create it
    /// should use [`HostFile::rebuild`] instead of editing by hand.
    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path_for(root);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading host file {}", path.display()))?;
        let file: HostFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing host file {}", path.display()))?;
        if file.version != HOST_FILE_VERSION {
            bail!(
                "host file {} has version {}, this binary reads version {}",
                path.display(),
                file.version,
                HOST_FILE_VERSION
            );
        }
        let want = canonical_or_plain(root);
        let have = canonical_or_plain(&file.data_root);
        if want != have {
            bail!(
                "host file {} names data root {}, but was loaded for {}: \
                 copy the world directories and rebuild instead",
                path.display(),
                file.data_root.display(),
                root.display()
            );
        }
        Ok(file)
    }

    /// Save atomically: write temp file, fsync, rename over the target.
    ///
    /// # Errors
    ///
    /// Fails on any filesystem error; the previous host file is left
    /// untouched (rename is atomic on the same filesystem).
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path_for(root);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).context("serializing host file")?;
        std::fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
        let f =
            std::fs::File::open(&tmp).with_context(|| format!("reopening {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
        drop(f);
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        // Make the rename itself durable: without it, a power loss can
        // bring back the previous host file after this save returned.
        #[cfg(unix)]
        std::fs::File::open(root)
            .and_then(|dir| dir.sync_all())
            .with_context(|| format!("fsync data root {}", root.display()))?;
        Ok(())
    }

    /// Rebuild from the directories under `root`: every immediate
    /// subdirectory holding a `journal.jsonl` becomes a world, defaulting
    /// to [`LifecycleState::Asleep`] (no process is started by a rebuild).
    /// Directories without a journal are not worlds and are skipped. No
    /// journal data is read or modified: rebuild loses no world and no
    /// journal bytes by construction.
    pub fn rebuild(root: &Path) -> Result<Self> {
        let mut file = Self::empty(root);
        let entries = std::fs::read_dir(root)
            .with_context(|| format!("listing data root {}", root.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("listing data root {}", root.display()))?;
            if !entry
                .file_type()
                .with_context(|| format!("typing {}", entry.path().display()))?
                .is_dir()
            {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_name(&name).is_err() {
                continue;
            }
            if !entry.path().join("journal.jsonl").is_file() {
                continue;
            }
            file.worlds.insert(
                name,
                WorldRecord {
                    state: LifecycleState::Asleep,
                    ..WorldRecord::default()
                },
            );
        }
        Ok(file)
    }

    /// Record a validated transition, appending to history.
    fn transition(
        &mut self,
        name: &str,
        to: LifecycleState,
        actor: &str,
        at_secs: u64,
    ) -> Result<()> {
        let record = self
            .worlds
            .get_mut(name)
            .with_context(|| format!("unknown world {name:?}"))?;
        let prior = record.state;
        check_transition(name, prior, to)?;
        record.state = to;
        record.history.push(StateChange {
            at_secs,
            actor: actor.to_owned(),
            prior,
        });
        Ok(())
    }

    /// Pin `name` always-live, or unpin it.
    ///
    /// # Errors
    ///
    /// Fails for a world this file does not record.
    pub fn set_pinned(&mut self, name: &str, pinned: bool) -> Result<()> {
        self.record_mut(name)?.pinned = pinned;
        Ok(())
    }

    /// Set how `name` wakes from asleep.
    ///
    /// # Errors
    ///
    /// Fails for a world this file does not record.
    pub fn set_wake(&mut self, name: &str, wake: WakeMode) -> Result<()> {
        self.record_mut(name)?.wake = wake;
        Ok(())
    }

    /// Record the journal-bearing directory `root/name` as an asleep
    /// world adopted by `actor`.
    ///
    /// # Errors
    ///
    /// Fails for invalid names, names already recorded, missing
    /// directories and directories without a journal.
    pub fn adopt(&mut self, root: &Path, name: &str, actor: &str) -> Result<()> {
        validate_name(name)?;
        if self.worlds.contains_key(name) {
            bail!("world {name:?} is already in the host file");
        }
        let dir = root.join(name);
        if !dir.is_dir() {
            bail!("no world directory at {}", dir.display());
        }
        if !dir.join("journal.jsonl").is_file() {
            bail!(
                "{} has no journal.jsonl: not a world directory",
                dir.display()
            );
        }
        self.worlds.insert(
            name.to_owned(),
            WorldRecord {
                state: LifecycleState::Asleep,
                history: vec![StateChange {
                    at_secs: now_secs(),
                    actor: actor.to_owned(),
                    prior: LifecycleState::Asleep,
                }],
                ..WorldRecord::default()
            },
        );
        Ok(())
    }

    fn record_mut(&mut self, name: &str) -> Result<&mut WorldRecord> {
        let root = self.data_root.display().to_string();
        self.worlds
            .get_mut(name)
            .with_context(|| format!("no world {name:?} in data root {root}"))
    }
}

/// Validate a world name before it becomes a directory name.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        bail!("world name must be 1..=128 bytes");
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("world name {name:?} is not a plain directory name");
    }
    if name == HOST_FILE_NAME {
        bail!("world name {name:?} is reserved for the host file");
    }
    if name == SNAPSHOT_DIR_NAME {
        bail!("world name {name:?} is reserved for snapshot sets");
    }
    Ok(())
}

/// Is `to` reachable from `from` by one operator act?
fn check_transition(name: &str, from: LifecycleState, to: LifecycleState) -> Result<()> {
    if from == to {
        bail!("world {name:?} is already {from}");
    }
    match (from, to) {
        (LifecycleState::Asleep, LifecycleState::Live) => Ok(()),
        (LifecycleState::Live, LifecycleState::Asleep) => Ok(()),
        (LifecycleState::Asleep, LifecycleState::Archived) => Ok(()),
        (LifecycleState::Archived, LifecycleState::Asleep) => Ok(()),
        (LifecycleState::Archived, LifecycleState::Live) => {
            bail!("world {name:?} is archived: restore to asleep before starting")
        }
        (LifecycleState::Live, LifecycleState::Archived) => {
            bail!("world {name:?} is live: stop to asleep before archiving")
        }
        (from, to) => {
            bail!("world {name:?} cannot move from {from} to {to}")
        }
    }
}

fn canonical_or_plain(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One world's per-state storage row for the operator surface.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct WorldCensus {
    /// World name (directory name).
    pub name: String,
    /// Lifecycle state at census time.
    pub state: LifecycleState,
    /// `journal.jsonl` + siblings.
    pub journal_bytes: u64,
    /// `modules/<blake3>.wasm`.
    pub modules_bytes: u64,
    /// Every other regular file under the world directory.
    pub other_bytes: u64,
    /// Snapshot sets kept for this world (#13739).
    pub snapshot_sets: u64,
    /// Their total size: custody cost on top of the world directory.
    pub snapshot_bytes: u64,
    /// The world directory was missing when inventoried.
    pub missing: bool,
}

/// Totals for one lifecycle state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StateCensus {
    /// Worlds in this state.
    pub worlds: u64,
    /// Sum of `journal.jsonl` bytes.
    pub journal_bytes: u64,
    /// Sum of `modules/` bytes.
    pub modules_bytes: u64,
    /// Sum of all other bytes.
    pub other_bytes: u64,
    /// Sum of snapshot set bytes, counted against this state's capacity.
    pub snapshot_bytes: u64,
}

/// Per-state counts + bytes for the operator surface (packet P4 input).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Census {
    /// One row per world in the host file.
    pub worlds: Vec<WorldCensus>,
    /// Aggregate for live worlds.
    pub live: StateCensus,
    /// Aggregate for asleep worlds.
    pub asleep: StateCensus,
    /// Aggregate for archived worlds.
    pub archived: StateCensus,
}

impl Census {
    fn add(&mut self, row: WorldCensus) {
        let slot = match row.state {
            LifecycleState::Live => &mut self.live,
            LifecycleState::Asleep => &mut self.asleep,
            LifecycleState::Archived => &mut self.archived,
        };
        slot.worlds += 1;
        slot.journal_bytes = slot.journal_bytes.saturating_add(row.journal_bytes);
        slot.modules_bytes = slot.modules_bytes.saturating_add(row.modules_bytes);
        slot.other_bytes = slot.other_bytes.saturating_add(row.other_bytes);
        slot.snapshot_bytes = slot.snapshot_bytes.saturating_add(row.snapshot_bytes);
        self.worlds.push(row);
    }
}

/// Per-state counts + bytes over the worlds `file` records under `root`,
/// without opening any world for serving. Missing directories census as
/// zero-byte rows, never as errors.
#[must_use]
pub fn census_of(root: &Path, file: &HostFile) -> Census {
    let mut census = Census::default();
    let snapshot_root = file.snapshot_root(root);
    for (name, record) in &file.worlds {
        let dir = root.join(name);
        let snapshots = usage(&snapshot_root, name);
        let mut row = WorldCensus {
            name: name.clone(),
            state: record.state,
            snapshot_sets: snapshots.sets,
            snapshot_bytes: snapshots.bytes,
            ..WorldCensus::default()
        };
        match inventory(&dir, WalkLimits::default()) {
            Ok(inv) => {
                row.journal_bytes = inv.journal.bytes;
                row.modules_bytes = inv.modules.bytes;
                row.other_bytes = inv.other.bytes;
            }
            Err(_) => {
                row.missing = true;
            }
        }
        census.add(row);
    }
    census.worlds.sort_by(|a, b| a.name.cmp(&b.name));
    census
}

/// Actor recorded when a client's connection wakes an asleep world.
pub const WAKE_ON_CONNECT: &str = "wake-on-connect";

/// Actor recorded when the serving hub closes an idle world.
pub const IDLE_CLOSE: &str = "idle";

/// Actor recorded when an admin-authorised request creates a new world in
/// a served data root.
pub const ADMIN_CREATE: &str = "admin-create";

/// The host file of a data root, shared by a serving process that opens
/// worlds on demand (#10693 serve-attach). A client naming an asleep
/// world wakes it ([`HostLifecycle::admit_wake`]); the hub closing an
/// idle world records it asleep again ([`HostLifecycle::record_closed`]).
/// Archived and unknown worlds are refused: waking never restores an
/// archived world and never creates one. Every change is saved before it
/// is reported.
#[derive(Debug)]
pub struct HostLifecycle {
    root: PathBuf,
    file: std::sync::Mutex<HostFile>,
    /// The host file on disk as this process last read or wrote it, so an
    /// operator's edit made by another process (`id host pin` with no
    /// host answering the control socket) is adopted on the next read.
    seen: std::sync::Mutex<Option<FileStamp>>,
}

/// Enough of a file's metadata to tell that it was replaced: every save
/// renames a new file over the old one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
    inode: u64,
}

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some(FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
        inode,
    })
}

/// What [`HostLifecycle::admit_wake`] did, so a failed open can be undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The world was already recorded live; nothing changed.
    AlreadyLive,
    /// The world was asleep and is now recorded live.
    Woken,
    /// The world was new and is now recorded live by [`ADMIN_CREATE`].
    Created,
}

impl HostLifecycle {
    /// Load the host file for `root`, rebuilding and saving it when absent.
    ///
    /// # Errors
    ///
    /// As [`HostFile::load`], [`HostFile::rebuild`] and [`HostFile::save`].
    pub fn open(root: &Path) -> Result<Self> {
        let file = if HostFile::path_for(root).is_file() {
            HostFile::load(root)?
        } else {
            let file = HostFile::rebuild(root)?;
            file.save(root)?;
            file
        };
        Ok(Self {
            root: root.to_path_buf(),
            file: std::sync::Mutex::new(file),
            seen: std::sync::Mutex::new(stamp_of(&HostFile::path_for(root))),
        })
    }

    /// Whether the operator pinned `name` always-live.
    pub fn pinned(&self, name: &str) -> bool {
        self.lock()
            .worlds
            .get(name)
            .is_some_and(|record| record.pinned)
    }

    /// Pin `name` always-live, or unpin it, and save.
    ///
    /// # Errors
    ///
    /// Fails for a world the host file does not know, and when the save
    /// fails (nothing changes then).
    pub fn set_pinned(&self, name: &str, pinned: bool) -> Result<()> {
        let mut file = self.lock();
        let mut next = file.clone();
        next.set_pinned(name, pinned)?;
        self.save(&next)?;
        *file = next;
        Ok(())
    }

    /// Set how `name` wakes from asleep, and save.
    ///
    /// # Errors
    ///
    /// As [`HostLifecycle::set_pinned`].
    pub fn set_wake(&self, name: &str, wake: WakeMode) -> Result<()> {
        let mut file = self.lock();
        let mut next = file.clone();
        next.set_wake(name, wake)?;
        self.save(&next)?;
        *file = next;
        Ok(())
    }

    /// The data root this host file describes.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The recorded state of `name`, if the host file knows it.
    pub fn state(&self, name: &str) -> Option<LifecycleState> {
        self.lock().worlds.get(name).map(|record| record.state)
    }

    /// A copy of the host file as it stands.
    pub fn snapshot(&self) -> HostFile {
        self.lock().clone()
    }

    /// Admit a client's connection to `name`: an asleep world is recorded
    /// live by [`WAKE_ON_CONNECT`], a live one is left as it is.
    ///
    /// # Errors
    ///
    /// Refuses an archived world (restore it first), an asleep world whose
    /// wake mode is [`WakeMode::Operator`] (an operator starts it), a
    /// world the host file does not know, and a failed save (nothing is
    /// recorded then).
    pub fn admit_wake(&self, name: &str) -> Result<Admission> {
        let mut file = self.lock();
        self.admit_known(&mut file, name)
    }

    /// Admit an admin-authorised request that may create `name`: a world
    /// the host file does not know is recorded live by [`ADMIN_CREATE`];
    /// a known one is admitted as [`HostLifecycle::admit_wake`] admits it,
    /// so creating never restores an archived world.
    ///
    /// # Errors
    ///
    /// As [`HostLifecycle::admit_wake`] for a known world; for a new one,
    /// refuses a name that cannot be a world directory, and a failed save.
    pub fn admit_create(&self, name: &str) -> Result<Admission> {
        let mut file = self.lock();
        if file.worlds.contains_key(name) {
            return self.admit_known(&mut file, name);
        }
        validate_name(name)?;
        let mut next = file.clone();
        next.worlds.insert(
            name.to_owned(),
            WorldRecord {
                state: LifecycleState::Live,
                history: vec![StateChange {
                    at_secs: now_secs(),
                    actor: ADMIN_CREATE.to_owned(),
                    prior: LifecycleState::Asleep,
                }],
                ..WorldRecord::default()
            },
        );
        self.save(&next)?;
        *file = next;
        Ok(Admission::Created)
    }

    fn admit_known(&self, file: &mut HostFile, name: &str) -> Result<Admission> {
        match file.worlds.get(name).map(|record| record.state) {
            None => bail!("no world {name:?} in this data root"),
            Some(LifecycleState::Live) => Ok(Admission::AlreadyLive),
            Some(LifecycleState::Archived) => {
                bail!("world {name:?} is archived: an operator must restore it before it wakes")
            }
            Some(LifecycleState::Asleep)
                if file
                    .worlds
                    .get(name)
                    .is_some_and(|record| record.wake == WakeMode::Operator) =>
            {
                bail!("world {name:?} wakes only when an operator starts it")
            }
            Some(LifecycleState::Asleep) => {
                self.record(file, name, LifecycleState::Live, WAKE_ON_CONNECT)?;
                Ok(Admission::Woken)
            }
        }
    }

    /// Undo an [`Admission::Woken`] whose open then failed: the world goes
    /// back to asleep with the wake's history entry removed, as if the
    /// connection never arrived. An [`Admission::Created`] world is
    /// forgotten again. [`Admission::AlreadyLive`] undoes nothing.
    ///
    /// # Errors
    ///
    /// Fails when the save fails; the in-memory record is restored anyway.
    pub fn undo_wake(&self, name: &str, admission: Admission) -> Result<()> {
        if admission == Admission::AlreadyLive {
            return Ok(());
        }
        let mut file = self.lock();
        if admission == Admission::Created {
            let only_created = file.worlds.get(name).is_some_and(|record| {
                record.state == LifecycleState::Live
                    && record.history.len() == 1
                    && record.history[0].actor == ADMIN_CREATE
            });
            if !only_created {
                return Ok(());
            }
            file.worlds.remove(name);
            return self.save(&file);
        }
        let Some(record) = file.worlds.get_mut(name) else {
            return Ok(());
        };
        let woke_last = record.history.last().is_some_and(|change| {
            change.actor == WAKE_ON_CONNECT && change.prior == LifecycleState::Asleep
        });
        if record.state != LifecycleState::Live || !woke_last {
            return Ok(());
        }
        record.state = LifecycleState::Asleep;
        record.history.pop();
        self.save(&file)
    }

    /// The hub closed `name` (idle, or to make room): record it asleep by
    /// [`IDLE_CLOSE`]. A world not recorded live is left as it is.
    ///
    /// # Errors
    ///
    /// Fails when the save fails; nothing is recorded then.
    pub fn record_closed(&self, name: &str) -> Result<()> {
        let mut file = self.lock();
        if file.worlds.get(name).map(|record| record.state) != Some(LifecycleState::Live) {
            return Ok(());
        }
        self.record(&mut file, name, LifecycleState::Asleep, IDLE_CLOSE)
    }

    /// Apply one transition to a copy, save it, then adopt it, so a failed
    /// save leaves both the file on disk and the memory unchanged.
    fn record(
        &self,
        file: &mut HostFile,
        name: &str,
        to: LifecycleState,
        actor: &str,
    ) -> Result<()> {
        let mut next = file.clone();
        next.transition(name, to, actor, now_secs())?;
        self.save(&next)?;
        *file = next;
        Ok(())
    }

    /// The host file, first adopting it from disk when another process
    /// replaced it since this one last read or wrote it. A replacement
    /// that does not load is ignored and overwritten by the next save.
    fn lock(&self) -> std::sync::MutexGuard<'_, HostFile> {
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = stamp_of(&HostFile::path_for(&self.root));
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if now.is_some() && now != *seen {
            if let Ok(fresh) = HostFile::load(&self.root) {
                *file = fresh;
            }
            *seen = now;
        }
        file
    }

    /// Save `file` and remember the result as this process's own write.
    fn save(&self, file: &HostFile) -> Result<()> {
        file.save(&self.root)?;
        *self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            stamp_of(&HostFile::path_for(&self.root));
        Ok(())
    }
}

/// One world's facts for `id host inspect` (#10696), read from the host
/// file and the world directory without opening the world for serving.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldInspection {
    /// World name (directory name).
    pub name: String,
    /// The data root the host file describes.
    pub data_root: PathBuf,
    /// The world's directory under the data root.
    pub dir: PathBuf,
    /// Recorded lifecycle state.
    pub state: LifecycleState,
    /// Recorded transitions, oldest first.
    pub history: Vec<StateChange>,
    /// Pinned always-live by an operator.
    pub pinned: bool,
    /// How the world wakes from asleep.
    pub wake: WakeMode,
    /// The journal's entry count, version and torn tail, when it parses.
    pub journal: Option<JournalFacts>,
    /// Why the journal could not be read, when it could not.
    pub journal_error: Option<String>,
    /// Module files under `modules/`.
    pub module_count: u64,
    /// The world's storage row, as `id host list` shows it.
    pub storage: WorldCensus,
    /// What the last restart replay found, when there was one.
    pub last_recovery: Option<RecoveryRecord>,
    /// The ceilings `id host run` replays and serves the world under.
    pub ceilings: HostCeilings,
}

/// The ceilings a [`Supervisor`] replays and serves every world under:
/// the per-world bounds and the world's whole memory budget (#10696).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HostCeilings {
    /// Most active capabilities in the world.
    pub participants: u64,
    /// Longest chat message, in bytes.
    pub chat_bytes: u64,
    /// Largest game input, in bytes.
    pub input_bytes: u64,
    /// Largest presentation text a program returns, in bytes.
    pub presentation_bytes: u64,
    /// Largest snapshot frame, in bytes.
    pub snapshot_bytes: u64,
    /// Recent events kept for reconnect catch-up.
    pub retained_events: u64,
    /// Events between program checkpoints; `0` turns checkpoints off.
    pub checkpoint_every: u64,
    /// The world's whole memory budget, in bytes.
    pub total_memory_bytes: u64,
}

impl HostCeilings {
    /// The ceilings that `world` and `runtime` set.
    pub fn of(world: &WorldLimits, runtime: &RuntimeLimits) -> Self {
        let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        Self {
            participants: wide(world.participants),
            chat_bytes: wide(world.chat_bytes),
            input_bytes: wide(world.input_bytes),
            presentation_bytes: wide(world.presentation_bytes),
            snapshot_bytes: wide(world.snapshot_bytes),
            retained_events: wide(world.retained_events),
            checkpoint_every: world.checkpoint_every,
            total_memory_bytes: runtime.total_memory_bytes,
        }
    }

    /// The ceilings a [`Supervisor`] uses.
    pub fn host() -> Self {
        let (world, runtime) = host_limits();
        Self::of(&world, &runtime)
    }
}

/// The limits a [`Supervisor`] replays and serves every world under.
fn host_limits() -> (WorldLimits, RuntimeLimits) {
    (WorldLimits::default(), RuntimeLimits::default())
}

/// Inspect `name` in the data root `root` that `file` describes, reading
/// only the host file record and the world directory: nothing is opened
/// for serving, replayed or written.
///
/// # Errors
///
/// Fails when the host file has no world called `name`. A journal that
/// does not parse is reported in [`WorldInspection::journal_error`], not
/// as an error, so a damaged world can still be inspected.
pub fn inspect_world(root: &Path, file: &HostFile, name: &str) -> Result<WorldInspection> {
    let record = file
        .worlds
        .get(name)
        .with_context(|| format!("no world {name:?} in data root {}", root.display()))?;
    let dir = root.join(name);
    let (journal, journal_error) = match journal_facts(&dir.join(JOURNAL_FILE)) {
        Ok(facts) => (Some(facts), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let module_count = inventory(&dir, WalkLimits::default())
        .map(|inv| inv.modules.files)
        .unwrap_or(0);
    let storage = census_of(root, file)
        .worlds
        .into_iter()
        .find(|row| row.name == name)
        .unwrap_or_default();
    Ok(WorldInspection {
        name: name.to_owned(),
        data_root: file.data_root.clone(),
        dir,
        state: record.state,
        history: record.history.clone(),
        pinned: record.pinned,
        wake: record.wake,
        journal,
        journal_error,
        module_count,
        storage,
        last_recovery: record.last_recovery.clone(),
        ceilings: HostCeilings::host(),
    })
}

/// The answer body for a pin or wake-mode change: the world's settings as
/// the host file now records them.
///
/// # Errors
///
/// Fails for a world the file does not record.
pub fn settings_body(file: &HostFile, world: &str) -> Result<serde_json::Value> {
    let record = file
        .worlds
        .get(world)
        .with_context(|| format!("no world {world:?} in this data root"))?;
    Ok(serde_json::json!({
        "world": world,
        "state": record.state,
        "pinned": record.pinned,
        "wake": record.wake,
    }))
}

/// The answer body for a snapshot-policy change: the policy `world`
/// follows (or the host-wide default when `world` is `None`), where sets
/// live, and the world's sets on disk.
///
/// # Errors
///
/// Fails for a world the file does not record.
pub fn snapshot_settings_body(
    root: &Path,
    file: &HostFile,
    world: Option<&str>,
) -> Result<serde_json::Value> {
    let snapshot_root = file.snapshot_root(root);
    let Some(name) = world else {
        return Ok(serde_json::json!({
            "world": null,
            "policy": file.snapshots.unwrap_or(SnapshotPolicy::OFF),
            "dir": snapshot_root,
        }));
    };
    let record = file
        .worlds
        .get(name)
        .with_context(|| format!("no world {name:?} in this data root"))?;
    let sets = list_sets(&snapshot_root, name)?;
    Ok(serde_json::json!({
        "world": name,
        "state": record.state,
        "policy": file.snapshot_policy(name),
        "own_policy": record.snapshots.is_some(),
        "dir": world_snapshot_dir(&snapshot_root, name),
        "sets": sets,
    }))
}

/// Name of the host's signing key inside a data root: the Ed25519 key
/// that signs world exports, readable by its owner alone.
pub const HOST_KEY_NAME: &str = "host.key";

/// Restore the export at `path` into `root/<world>`, the world the
/// export names, without recording it.
async fn restore_export_dir(
    root: &Path,
    file: &HostFile,
    path: &Path,
    pinned_source: [u8; 32],
) -> Result<ExportManifest> {
    let manifest = read_export_manifest(path).await?;
    validate_name(&manifest.world)?;
    if file.worlds.contains_key(&manifest.world) {
        bail!(
            "world {:?} is already in this data root: archive or move it before restoring",
            manifest.world
        );
    }
    import_world_from_file(path, &root.join(&manifest.world), pinned_source).await
}

/// Restore the export file `path`, signed by `pinned_source`, into the data
/// root `root` with no host running: the world directory is written and
/// the host file records the world asleep, adopted by `actor`.
///
/// # Errors
///
/// Fails for a world name that cannot be a directory, a world the host
/// file already records, an existing world directory, and as
/// [`import_world_from_file`]; the host file is unchanged then.
pub async fn import_into_root(
    root: &Path,
    path: &Path,
    pinned_source: [u8; 32],
    actor: &str,
) -> Result<ExportManifest> {
    let mut file = HostFile::read_or_rebuild(root)?;
    let manifest = restore_export_dir(root, &file, path, pinned_source).await?;
    file.adopt(root, &manifest.world, actor)?;
    file.save(root)?;
    Ok(manifest)
}

/// A live world held by the [`Supervisor`]: its service plus the replay
/// report from the `start` that brought it live.
#[derive(Clone, Debug)]
pub struct LiveWorld {
    /// The replayed, held world.
    pub service: WorldService,
    /// What the start-time replay found.
    pub report: OpenReport,
}

/// The refusal anyone but the operator gets for a restore.
fn restore_refusal() -> anyhow::Error {
    crate::directory::Refusal::Forbidden("only the operator restores an archived world".to_owned())
        .into()
}

/// A lifecycle change one authority may ask of a world (#10694).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleAct {
    /// Asleep to live.
    Wake,
    /// Live to asleep.
    Pause,
    /// Asleep to archived.
    Archive,
    /// Archived to asleep: the operator's alone.
    Restore,
}

impl LifecycleAct {
    /// How a refusal names the act.
    #[must_use]
    pub const fn phrase(self) -> &'static str {
        match self {
            Self::Wake => "wakes the world",
            Self::Pause => "pauses the world",
            Self::Archive => "archives the world",
            Self::Restore => "restores the world",
        }
    }
}

/// The supervisor: one data root, the host file beside it, and the held
/// live worlds. Created by [`Supervisor::open`], which replays every
/// world the host file records live.
#[derive(Debug)]
pub struct Supervisor {
    root: PathBuf,
    file: HostFile,
    live: HashMap<String, LiveWorld>,
    signing_key: Option<SigningKey>,
}

impl Supervisor {
    /// Open the data root: load the host file (rebuilding it when absent),
    /// then replay every world recorded live. Worlds recorded asleep or
    /// archived are untouched. A live world whose journal fails replay is
    /// left recorded live and reported: replay failure never silently
    /// demotes a world.
    ///
    /// # Errors
    ///
    /// Fails when the data root cannot be listed, the host file is
    /// corrupt or names another root, or any recorded-live world fails
    /// replay (the error names every failed world).
    pub async fn open(root: &Path) -> Result<Self> {
        let path = HostFile::path_for(root);
        let file = if path.is_file() {
            HostFile::load(root)?
        } else {
            let file = HostFile::rebuild(root)?;
            file.save(root)?;
            file
        };
        let mut supervisor = Self {
            root: root.to_path_buf(),
            file,
            live: HashMap::new(),
            signing_key: None,
        };
        let live_names: Vec<String> = supervisor
            .file
            .worlds
            .iter()
            .filter(|(_, record)| record.state == LifecycleState::Live)
            .map(|(name, _)| name.clone())
            .collect();
        let mut failures = Vec::new();
        let mut recovered = supervisor.file.clone();
        for name in &live_names {
            let torn_tail = journal_facts(&supervisor.root.join(name).join(JOURNAL_FILE))
                .is_ok_and(|facts| facts.torn_tail);
            let outcome = supervisor.replay_live(name).await;
            let record = match &outcome {
                Ok(report) => RecoveryRecord::replayed(now_secs(), torn_tail, report),
                Err(error) => RecoveryRecord::failed(now_secs(), torn_tail, format!("{error:#}")),
            };
            if let Some(world) = recovered.worlds.get_mut(name) {
                world.last_recovery = Some(record);
            }
            if let Err(error) = outcome {
                failures.push(format!("{name}: {error:#}"));
            }
        }
        if !live_names.is_empty() {
            recovered
                .save(&supervisor.root)
                .context("recording the restart replay in the host file")?;
            supervisor.file = recovered;
        }
        if !failures.is_empty() {
            bail!(
                "replay failed for {} live world(s): {}",
                failures.len(),
                failures.join("; ")
            );
        }
        Ok(supervisor)
    }

    /// The data root under supervision.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The current host file (states + history).
    pub fn file(&self) -> &HostFile {
        &self.file
    }

    /// Names of the worlds currently held live, sorted.
    pub fn live_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.live.keys().cloned().collect();
        names.sort();
        names
    }

    /// The replay report for a held-live world, if held.
    pub fn live_report(&self, name: &str) -> Option<&OpenReport> {
        self.live.get(name).map(|held| &held.report)
    }

    /// Adopt a journal-bearing directory as an asleep world.
    ///
    /// # Errors
    ///
    /// Fails for unknown/invalid names, missing directories, directories
    /// without a journal, and names already in the host file.
    pub fn adopt(&mut self, name: &str, actor: &str) -> Result<()> {
        let mut next = self.file.clone();
        next.adopt(&self.root, name, actor)?;
        next.save(&self.root)?;
        self.file = next;
        Ok(())
    }

    /// Use `key` to sign world exports ([`Supervisor::export`]).
    pub fn set_signing_key(&mut self, key: SigningKey) {
        self.signing_key = Some(key);
    }

    /// Export a live world to the new file `path`: its journal, directory
    /// facts and active module as signed replication frames
    /// ([`crate::world_export`]). The world keeps serving.
    ///
    /// # Errors
    ///
    /// Fails for a world not held live (start it first), when the host
    /// has no signing key, for an existing `path`, and as
    /// [`export_world_to_file`].
    pub async fn export(&self, name: &str, path: &Path) -> Result<ExportManifest> {
        let held = self
            .live
            .get(name)
            .with_context(|| format!("world {name:?} is not live: start it before exporting"))?;
        let key = self
            .signing_key
            .as_ref()
            .context("this host has no signing key for exports")?;
        export_world_to_file(&held.service, key, path).await
    }

    /// Set the host-wide snapshot policy, or one world's own, and save
    /// (#13739). The running host follows it from its next check.
    ///
    /// # Errors
    ///
    /// As [`HostFile::set_snapshot_policy`], and when the save fails
    /// (nothing changes then).
    pub fn set_snapshot_policy(
        &mut self,
        world: Option<&str>,
        policy: Option<SnapshotPolicy>,
    ) -> Result<()> {
        let mut next = self.file.clone();
        next.set_snapshot_policy(world, policy)?;
        next.save(&self.root)?;
        self.file = next;
        Ok(())
    }

    /// Take a set for every live world whose policy says it is due at
    /// `now_secs`, then prune each world's sets to its policy (#13739).
    ///
    /// A world whose journal has not grown since its newest set gets the
    /// fresh set in place of that one. A host with no signing key takes
    /// nothing. One world failing is recorded in the run and never stops
    /// the others; a failed export leaves no partial set behind.
    pub async fn run_due_snapshots(&self, now_secs: u64) -> SnapshotRun {
        let mut run = SnapshotRun::default();
        if self.signing_key.is_none() {
            return run;
        }
        let snapshot_root = self.file.snapshot_root(&self.root);
        let mut names: Vec<&String> = self.live.keys().collect();
        names.sort();
        for name in names {
            let policy = self.file.snapshot_policy(name);
            if !policy.is_on() {
                continue;
            }
            if let Err(error) = self
                .snapshot_world(name, &policy, &snapshot_root, now_secs, &mut run)
                .await
            {
                run.failed.push((name.clone(), format!("{error:#}")));
            }
        }
        run
    }

    async fn snapshot_world(
        &self,
        name: &str,
        policy: &SnapshotPolicy,
        snapshot_root: &Path,
        now_secs: u64,
        run: &mut SnapshotRun,
    ) -> Result<()> {
        let sets = list_sets(snapshot_root, name)?;
        let previous = sets.last().cloned();
        if !is_due(policy, previous.as_ref(), now_secs) {
            return Ok(());
        }
        let dir = world_snapshot_dir(snapshot_root, name);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating snapshot directory {}", dir.display()))?;
        // The entry count is known only once the export is written, so it
        // is written under a name that is not a set, then renamed.
        let taking = dir.join(format!(".{now_secs:020}.taking"));
        let manifest = self.export(name, &taking).await?;
        let path = dir.join(set_file_name(now_secs, manifest.entries));
        std::fs::rename(&taking, &path)
            .with_context(|| format!("renaming {} to {}", taking.display(), path.display()))?;
        let bytes = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        let taken = SnapshotSet {
            world: name.to_owned(),
            path,
            at_secs: now_secs,
            entries: manifest.entries,
            bytes,
        };
        run.taken.push(taken.clone());
        let mut after = sets;
        // The journal is append-only, so the same entry count is the same
        // journal: the new set replaces the newest one.
        if let Some(previous) = previous.filter(|set| set.entries == manifest.entries) {
            remove_sets(std::slice::from_ref(&previous))?;
            after.pop();
            run.removed.push(previous);
        }
        after.push(taken);
        let plan = prune_plan(&after, policy, now_secs);
        remove_sets(&plan)?;
        run.removed.extend(plan);
        Ok(())
    }

    /// Restore the export file `path`, signed by `pinned_source`, as a new
    /// asleep world named by the export, recorded by `actor`.
    ///
    /// # Errors
    ///
    /// As [`import_into_root`]; nothing is recorded when it fails.
    pub async fn import(
        &mut self,
        path: &Path,
        pinned_source: [u8; 32],
        actor: &str,
    ) -> Result<ExportManifest> {
        let manifest = restore_export_dir(&self.root, &self.file, path, pinned_source).await?;
        self.adopt(&manifest.world, actor)?;
        Ok(manifest)
    }

    /// Start an asleep world: journal replay, then hold live and record.
    ///
    /// # Errors
    ///
    /// Fails for archived worlds (restore first), live worlds, unknown
    /// names, and replay failures (the world stays asleep).
    pub async fn start(&mut self, name: &str, actor: &str) -> Result<OpenReport> {
        let at_secs = now_secs();
        self.file
            .transition(name, LifecycleState::Live, actor, at_secs)?;
        match self.replay_live(name).await {
            Ok(report) => {
                self.file.save(&self.root)?;
                Ok(report)
            }
            Err(error) => {
                // Roll back the recorded transition: the world never went
                // live, so the host file must keep saying asleep.
                if let Some(record) = self.file.worlds.get_mut(name) {
                    record.state = LifecycleState::Asleep;
                    record.history.pop();
                }
                Err(error).with_context(|| format!("starting world {name:?}"))?
            }
        }
    }

    /// Stop a live world: shut the actor down (drain), then record asleep.
    ///
    /// # Errors
    ///
    /// Fails for worlds that are not live or not held.
    pub async fn stop(&mut self, name: &str, actor: &str) -> Result<()> {
        let held = self
            .live
            .remove(name)
            .with_context(|| format!("world {name:?} is not held live"))?;
        held.service
            .world()
            .shutdown()
            .await
            .with_context(|| format!("shutting down world {name:?}"))?;
        self.file
            .transition(name, LifecycleState::Asleep, actor, now_secs())?;
        self.file.save(&self.root)?;
        Ok(())
    }

    /// Archive an asleep world. Live worlds must be stopped first;
    /// archived worlds refuse to start until restored.
    pub fn archive(&mut self, name: &str, actor: &str) -> Result<()> {
        self.file
            .transition(name, LifecycleState::Archived, actor, now_secs())?;
        self.file.save(&self.root)?;
        Ok(())
    }

    /// Restore an archived world to asleep (operator act, recorded).
    pub fn restore(&mut self, name: &str, actor: &str) -> Result<()> {
        self.file
            .transition(name, LifecycleState::Asleep, actor, now_secs())?;
        self.file.save(&self.root)?;
        Ok(())
    }

    /// Pin `name` always-live, or unpin it, and save.
    ///
    /// # Errors
    ///
    /// Fails for a world the host file does not know, and when the save
    /// fails (nothing changes then).
    pub fn set_pinned(&mut self, name: &str, pinned: bool) -> Result<()> {
        let mut next = self.file.clone();
        next.set_pinned(name, pinned)?;
        next.save(&self.root)?;
        self.file = next;
        Ok(())
    }

    /// Set how `name` wakes from asleep, and save.
    ///
    /// # Errors
    ///
    /// As [`Supervisor::set_pinned`].
    pub fn set_wake(&mut self, name: &str, wake: WakeMode) -> Result<()> {
        let mut next = self.file.clone();
        next.set_wake(name, wake)?;
        next.save(&self.root)?;
        self.file = next;
        Ok(())
    }

    /// Whether `authority` may take `act` on `name`, evaluated against the
    /// world's own authority data exactly as a local host evaluates it
    /// (#10694, design §4): the operator (the process admin) may take
    /// every act; the world's owner (an admin seat in one of its groups)
    /// may wake, pause and archive; restore is the operator's alone.
    /// Everyone else gets the world's forbidden refusal. A world that is
    /// not live is opened from its journal to ask and shut again, unrecorded.
    ///
    /// # Errors
    ///
    /// The refusal, or a failure to open the world to ask.
    pub async fn authorize(
        &self,
        name: &str,
        act: LifecycleAct,
        authority: &Authority,
    ) -> Result<()> {
        if *authority == Authority::Admin {
            return Ok(());
        }
        if act == LifecycleAct::Restore {
            return Err(restore_refusal());
        }
        if let Some(held) = self.live.get(name) {
            return held
                .service
                .world()
                .authorize_lifecycle(authority.clone(), act.phrase())
                .await;
        }
        let (service, _) = self.open_unrecorded(name).await?;
        let verdict = service
            .world()
            .authorize_lifecycle(authority.clone(), act.phrase())
            .await;
        service
            .world()
            .shutdown()
            .await
            .with_context(|| format!("closing world {name:?} after asking it"))?;
        verdict
    }

    /// Take `act` on `name` for `authority`, recorded as `actor`, once
    /// [`Self::authorize`] allows it. Returns the replay report of a wake.
    ///
    /// # Errors
    ///
    /// The refusal (nothing changes), or as the act itself.
    pub async fn act_as(
        &mut self,
        name: &str,
        act: LifecycleAct,
        authority: &Authority,
        actor: &str,
    ) -> Result<Option<OpenReport>> {
        self.authorize(name, act, authority).await?;
        match act {
            LifecycleAct::Wake => self.start(name, actor).await.map(Some),
            LifecycleAct::Pause => self.stop(name, actor).await.map(|()| None),
            LifecycleAct::Archive => self.archive(name, actor).map(|()| None),
            LifecycleAct::Restore => self.restore(name, actor).map(|()| None),
        }
    }

    /// Stop every held world, newest-first by name for determinism.
    pub async fn stop_all(&mut self, actor: &str) -> Result<()> {
        let mut names: Vec<String> = self.live.keys().cloned().collect();
        names.sort();
        for name in names {
            self.stop(&name, actor).await?;
        }
        Ok(())
    }

    /// Shut the host down: drain every held world (actor shutdown, so
    /// each journal's last append is fsynced and no input is in flight)
    /// while leaving the host file exactly as it stands. A world recorded
    /// live stays recorded live, so the next [`Supervisor::open`] replays
    /// it back to live: host restart returns live worlds to live, asleep
    /// to asleep and archived to archived (design §5). This is the host
    /// going away, not an operator stop; use [`Supervisor::stop`] or
    /// [`Supervisor::stop_all`] to record worlds asleep.
    ///
    /// Every held world is drained even when one fails, so one stuck
    /// world never leaves the others running.
    ///
    /// # Errors
    ///
    /// Fails after draining when any world's shutdown failed; the error
    /// names every failed world.
    pub async fn shutdown(mut self) -> Result<()> {
        let mut names: Vec<String> = self.live.keys().cloned().collect();
        names.sort();
        let mut failures = Vec::new();
        for name in names {
            if let Some(held) = self.live.remove(&name)
                && let Err(error) = held.service.world().shutdown().await
            {
                failures.push(format!("{name}: {error:#}"));
            }
        }
        if !failures.is_empty() {
            bail!(
                "shutdown failed for {} world(s): {}",
                failures.len(),
                failures.join("; ")
            );
        }
        Ok(())
    }

    /// Per-state counts + bytes over the worlds in the host file.
    /// Missing directories census as zero-byte rows, never as errors.
    pub fn census(&self) -> Census {
        census_of(&self.root, &self.file)
    }

    /// Replay `name`'s journal and hold the world live. The caller must
    /// already have validated (and, on success, recorded) the transition.
    /// Replay `name` from its journal without recording or holding it.
    async fn open_unrecorded(&self, name: &str) -> Result<(WorldService, OpenReport)> {
        validate_name(name)?;
        let dir = self.root.join(name);
        anyhow::ensure!(dir.is_dir(), "no world named {name:?} in this data root");
        let (world_limits, runtime_limits) = host_limits();
        open_world(
            &dir,
            name,
            world_limits,
            runtime_limits,
            |handle: WorldHandle| WorldService::new(handle, None),
        )
        .await
        .with_context(|| format!("replaying world {name:?}"))
    }

    async fn replay_live(&mut self, name: &str) -> Result<OpenReport> {
        let (service, report) = self.open_unrecorded(name).await?;
        self.live.insert(
            name.to_owned(),
            LiveWorld {
                service,
                report: report.clone(),
            },
        );
        Ok(report)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "host_parity_tests.rs"]
mod parity_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{Authority, Isolation};
    use tempfile::TempDir;

    fn actor() -> &'static str {
        "test-operator"
    }

    /// A data root with `names` world directories, each an empty journal.
    fn data_root_with(names: &[&str]) -> TempDir {
        let root = TempDir::new().unwrap();
        for name in names {
            let dir = root.path().join(name);
            std::fs::create_dir_all(dir.join("modules")).unwrap();
            std::fs::write(dir.join("journal.jsonl"), "").unwrap();
        }
        root
    }

    #[test]
    fn transition_matrix_matches_design_f1() {
        // (from, to, allowed)
        let cases = [
            (LifecycleState::Asleep, LifecycleState::Live, true),
            (LifecycleState::Live, LifecycleState::Asleep, true),
            (LifecycleState::Asleep, LifecycleState::Archived, true),
            (LifecycleState::Archived, LifecycleState::Asleep, true),
            (LifecycleState::Live, LifecycleState::Live, false),
            (LifecycleState::Asleep, LifecycleState::Asleep, false),
            (LifecycleState::Archived, LifecycleState::Archived, false),
            (LifecycleState::Archived, LifecycleState::Live, false),
            (LifecycleState::Live, LifecycleState::Archived, false),
        ];
        for (from, to, allowed) in cases {
            let result = check_transition("w", from, to);
            assert_eq!(
                result.is_ok(),
                allowed,
                "transition {from} -> {to} should be {}",
                if allowed { "allowed" } else { "refused" }
            );
        }
        let archived_err = check_transition("w", LifecycleState::Archived, LifecycleState::Live)
            .unwrap_err()
            .to_string();
        assert!(archived_err.contains("restore"), "{archived_err}");
        let live_err = check_transition("w", LifecycleState::Live, LifecycleState::Archived)
            .unwrap_err()
            .to_string();
        assert!(live_err.contains("stop"), "{live_err}");
    }

    #[test]
    fn host_file_roundtrip_preserves_states_and_history() {
        let root = data_root_with(&["a", "b"]);
        let mut file = HostFile::rebuild(root.path()).unwrap();
        file.transition("a", LifecycleState::Live, actor(), 1_700_000_000)
            .unwrap();
        file.transition("a", LifecycleState::Asleep, actor(), 1_700_000_001)
            .unwrap();
        file.transition("b", LifecycleState::Archived, actor(), 1_700_000_002)
            .unwrap();
        file.save(root.path()).unwrap();

        let loaded = HostFile::load(root.path()).unwrap();
        assert_eq!(loaded, file);
        assert_eq!(loaded.worlds["a"].history.len(), 2);
        assert_eq!(loaded.worlds["a"].history[0].prior, LifecycleState::Asleep);
        assert_eq!(loaded.worlds["b"].state, LifecycleState::Archived);
    }

    #[test]
    fn host_file_refuses_foreign_data_root() {
        let root = data_root_with(&["a"]);
        let mut file = HostFile::rebuild(root.path()).unwrap();
        file.save(root.path()).unwrap();
        let elsewhere = TempDir::new().unwrap();
        // Point the file at another root, then load for the original.
        file.data_root = elsewhere.path().to_path_buf();
        file.save(root.path()).unwrap();
        let err = HostFile::load(root.path()).unwrap_err().to_string();
        assert!(err.contains("names data root"), "{err}");
    }

    #[test]
    fn host_file_rejects_wrong_version() {
        let root = data_root_with(&["a"]);
        let mut file = HostFile::rebuild(root.path()).unwrap();
        file.version = HOST_FILE_VERSION + 1;
        file.save(root.path()).unwrap();
        assert!(HostFile::load(root.path()).is_err());
    }

    #[test]
    fn rebuild_adopts_journal_dirs_and_skips_the_rest() {
        let root = TempDir::new().unwrap();
        std::fs::create_dir_all(root.path().join("w1").join("modules")).unwrap();
        std::fs::write(root.path().join("w1").join("journal.jsonl"), "").unwrap();
        std::fs::create_dir_all(root.path().join("empty-dir")).unwrap();
        std::fs::write(root.path().join(HOST_FILE_NAME), "{}").unwrap();

        let file = HostFile::rebuild(root.path()).unwrap();
        assert!(file.worlds.contains_key("w1"));
        assert_eq!(file.worlds["w1"].state, LifecycleState::Asleep);
        assert!(!file.worlds.contains_key("empty-dir"));
        assert!(!file.worlds.contains_key(HOST_FILE_NAME));
    }

    #[test]
    fn world_names_must_be_plain_directory_names() {
        for bad in ["", ".", "..", "a/b", "a\\b", HOST_FILE_NAME] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(validate_name("burrow-1").is_ok());
    }

    #[tokio::test]
    async fn supervisor_replays_live_worlds_on_open() {
        let root = data_root_with(&["alpha", "beta"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup.live_names(), [] as [String; 0]);

        // Append a durable entry to alpha, then stop: the journal now
        // proves replay on the next open.
        sup.start("alpha", actor()).await.unwrap();
        let world = sup.live["alpha"].service.world();
        world
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        let before = sup.live_report("alpha").unwrap().journal_entries;
        assert_eq!(before, 1);
        sup.stop_all(actor()).await.unwrap();
        assert_eq!(sup.live_names(), [] as [String; 0]);
        drop(sup);

        // Clean restart: nothing is live, so nothing replays, and the
        // journal survives byte-identical.
        let journal_before =
            std::fs::read(root.path().join("alpha").join("journal.jsonl")).unwrap();
        let sup2 = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup2.live_names(), [] as [String; 0]);
        assert_eq!(sup2.file().worlds["alpha"].last_recovery, None);
        drop(sup2);
        assert_eq!(
            std::fs::read(root.path().join("alpha").join("journal.jsonl")).unwrap(),
            journal_before
        );

        // Crash simulation: a host file recording live with no running
        // holder (as a killed host would leave) replays on open and
        // reproduces the journal exactly.
        let mut crashed = HostFile::rebuild(root.path()).unwrap();
        crashed
            .transition("alpha", LifecycleState::Live, actor(), 1_700_000_100)
            .unwrap();
        crashed.save(root.path()).unwrap();
        let sup3 = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup3.live_names(), vec!["alpha".to_owned()]);
        let reopened = sup3.live_report("alpha").unwrap();
        assert!(reopened.journal_entries >= before);
        let recovery = sup3.file().worlds["alpha"].last_recovery.clone().unwrap();
        assert!(!recovery.torn_tail);
        assert_eq!(recovery.error, None);
        assert_eq!(recovery.sequence, reopened.sequence);
        let replayed = std::fs::read(root.path().join("alpha").join("journal.jsonl")).unwrap();
        assert!(replayed.starts_with(&journal_before));
    }

    #[tokio::test]
    async fn a_woken_world_serves_only_its_members() {
        use crate::world::{JoinCapability, WorldScopes};

        let root = data_root_with(&["alpha"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("alpha", actor()).await.unwrap();
        let world = sup.live["alpha"].service.world();
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        let (bo_id, bo) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
        world.chat(ann.clone(), "before sleep").await.unwrap();
        assert!(world.revoke(bo_id.id).await.unwrap());
        sup.stop("alpha", actor()).await.unwrap();
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Asleep);

        // A connection to the asleep world wakes it; membership comes back
        // from the journal before anything is served.
        sup.start("alpha", actor()).await.unwrap();
        let woken = sup.live["alpha"].service.world();
        assert!(
            woken
                .chat(JoinCapability::from_wire("not-a-member".into()), "hi")
                .await
                .is_err(),
            "a stranger's token means nothing after wake"
        );
        assert!(
            woken.chat(bo, "still here?").await.is_err(),
            "a member revoked before sleep stays revoked after wake"
        );
        assert!(woken.participant_id(ann.clone()).await.is_ok());
        woken.chat(ann, "after wake").await.unwrap();
        sup.stop_all(actor()).await.unwrap();
    }

    fn key_hex(seed: u8) -> String {
        use std::fmt::Write as _;
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .as_bytes()
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }

    async fn directory_as(
        service: &WorldService,
        key: &str,
        action: crate::directory_view::DirectoryAction,
    ) {
        let caller = crate::directory_auth::Caller {
            key: Some(key.to_owned()),
            ..crate::directory_auth::Caller::default()
        };
        service.directory(caller, action).await.unwrap();
    }

    fn forbidden(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<crate::directory::Refusal>(),
            Some(crate::directory::Refusal::Forbidden(_))
        )
    }

    #[tokio::test]
    async fn only_the_owner_or_the_operator_changes_a_world_lifecycle() {
        use crate::directory_view::DirectoryAction;

        let root = data_root_with(&["alpha"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("alpha", actor()).await.unwrap();
        let owner = key_hex(1);
        let guest = key_hex(2);
        {
            let service = &sup.live["alpha"].service;
            for (key, name) in [(&owner, "Owner"), (&guest, "Guest")] {
                directory_as(
                    service,
                    key,
                    DirectoryAction::SignUpKey { name: name.into() },
                )
                .await;
            }
            directory_as(
                service,
                &owner,
                DirectoryAction::CreateGroup {
                    name: "keepers".into(),
                    description: String::new(),
                    scopes: Vec::new(),
                },
            )
            .await;
        }
        let owner = Authority::Signed(owner);
        let guest = Authority::Signed(guest);

        // A guest may not pause a live world; nothing is recorded.
        let history = sup.file().worlds["alpha"].history.len();
        let refused = sup
            .act_as("alpha", LifecycleAct::Pause, &guest, "guest")
            .await
            .unwrap_err();
        assert!(forbidden(&refused), "{refused:#}");
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Live);
        assert_eq!(sup.file().worlds["alpha"].history.len(), history);

        // The owner may pause, archive and wake; restore is the operator's.
        sup.act_as("alpha", LifecycleAct::Pause, &owner, "owner")
            .await
            .unwrap();
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Asleep);
        let refused = sup
            .act_as("alpha", LifecycleAct::Wake, &guest, "guest")
            .await
            .unwrap_err();
        assert!(
            forbidden(&refused),
            "an asleep world asks its own directory"
        );
        assert!(sup.live_names().is_empty(), "a refused wake holds nothing");
        sup.act_as("alpha", LifecycleAct::Archive, &owner, "owner")
            .await
            .unwrap();
        let refused = sup
            .act_as("alpha", LifecycleAct::Restore, &owner, "owner")
            .await
            .unwrap_err();
        assert!(forbidden(&refused), "{refused:#}");
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Archived);
        sup.act_as("alpha", LifecycleAct::Restore, &Authority::Admin, actor())
            .await
            .unwrap();
        let report = sup
            .act_as("alpha", LifecycleAct::Wake, &owner, "owner")
            .await
            .unwrap();
        assert!(report.is_some());
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Live);
        sup.stop_all(actor()).await.unwrap();
    }

    #[tokio::test]
    async fn supervisor_tracks_two_worlds_in_different_states() {
        let root = data_root_with(&["live-one", "sleepy", "vault"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("live-one", actor()).await.unwrap();
        sup.archive("vault", actor()).unwrap();

        assert_eq!(sup.live_names(), vec!["live-one".to_owned()]);
        assert_eq!(sup.file().worlds["live-one"].state, LifecycleState::Live);
        assert_eq!(sup.file().worlds["sleepy"].state, LifecycleState::Asleep);
        assert_eq!(sup.file().worlds["vault"].state, LifecycleState::Archived);

        // Archived worlds refuse to start until restored.
        let err = sup.start("vault", actor()).await.unwrap_err().to_string();
        assert!(err.contains("restore"), "{err}");
        sup.restore("vault", actor()).unwrap();
        sup.start("vault", actor()).await.unwrap();
        assert_eq!(sup.file().worlds["vault"].state, LifecycleState::Live);
    }

    #[tokio::test]
    async fn clean_shutdown_returns_every_world_to_its_recorded_state() {
        let root = data_root_with(&["live-one", "sleepy", "vault"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("live-one", actor()).await.unwrap();
        sup.live
            .get("live-one")
            .unwrap()
            .service
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        sup.archive("vault", actor()).unwrap();
        let file_before = sup.file().clone();
        let journal_before =
            std::fs::read(root.path().join("live-one").join("journal.jsonl")).unwrap();
        assert_ne!(journal_before, [] as [u8; 0]);

        sup.shutdown().await.unwrap();

        // Shutdown records nothing: the host file is byte-for-byte the
        // file the running host last saved, and the drained journal is
        // what the world appended.
        assert_eq!(HostFile::load(root.path()).unwrap(), file_before);
        assert_eq!(
            std::fs::read(root.path().join("live-one").join("journal.jsonl")).unwrap(),
            journal_before
        );

        // Restart: live replays back to live, asleep and archived stay put.
        let sup2 = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup2.live_names(), vec!["live-one".to_owned()]);
        assert_eq!(sup2.file().worlds["live-one"].state, LifecycleState::Live);
        assert_eq!(sup2.file().worlds["sleepy"].state, LifecycleState::Asleep);
        assert_eq!(sup2.file().worlds["vault"].state, LifecycleState::Archived);
        assert!(sup2.live_report("live-one").unwrap().journal_entries >= 1);
        // The only change is the restart replay the host now records.
        let mut after = sup2.file().clone();
        let recovery = after
            .worlds
            .get_mut("live-one")
            .unwrap()
            .last_recovery
            .take()
            .unwrap();
        assert_eq!(recovery.error, None);
        assert_eq!(after, file_before);
    }

    #[tokio::test]
    async fn delete_and_rebuild_loses_no_world_or_journal_data() {
        let root = data_root_with(&["a", "b"]);
        let journal_a = std::fs::read(root.path().join("a").join("journal.jsonl")).unwrap();
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("a", actor()).await.unwrap();
        let world = sup.live["a"].service.world();
        world
            .set_isolation(Authority::Admin, Isolation::Isolated)
            .await
            .unwrap();
        sup.stop_all(actor()).await.unwrap();
        let journal_after = std::fs::read(root.path().join("a").join("journal.jsonl")).unwrap();
        assert!(journal_after.len() > journal_a.len());

        // Delete the host file and rebuild: both worlds survive with
        // byte-identical journals; states reset to the safe default.
        std::fs::remove_file(HostFile::path_for(root.path())).unwrap();
        let rebuilt = HostFile::rebuild(root.path()).unwrap();
        assert_eq!(rebuilt.worlds.len(), 2);
        assert_eq!(
            std::fs::read(root.path().join("a").join("journal.jsonl")).unwrap(),
            journal_after
        );
        assert!(
            rebuilt
                .worlds
                .values()
                .all(|r| r.state == LifecycleState::Asleep)
        );

        // The rebuilt file loads and supervises normally.
        rebuilt.save(root.path()).unwrap();
        let sup2 = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup2.file().worlds.len(), 2);
    }

    #[test]
    fn host_lifecycle_wakes_asleep_and_refuses_archived_and_unknown() {
        let root = data_root_with(&["sleepy", "shelved"]);
        let host = HostLifecycle::open(root.path()).unwrap();
        assert!(HostFile::path_for(root.path()).is_file());
        {
            let mut file = host.lock();
            file.transition("shelved", LifecycleState::Archived, actor(), 1)
                .unwrap();
        }

        assert_eq!(host.admit_wake("sleepy").unwrap(), Admission::Woken);
        assert_eq!(host.state("sleepy"), Some(LifecycleState::Live));
        assert_eq!(host.admit_wake("sleepy").unwrap(), Admission::AlreadyLive);

        let saved = HostFile::load(root.path()).unwrap();
        let record = &saved.worlds["sleepy"];
        assert_eq!(record.state, LifecycleState::Live);
        assert_eq!(record.history.len(), 1);
        assert_eq!(record.history[0].actor, WAKE_ON_CONNECT);
        assert_eq!(record.history[0].prior, LifecycleState::Asleep);

        let archived = host.admit_wake("shelved").unwrap_err().to_string();
        assert!(archived.contains("restore"), "{archived}");
        assert_eq!(host.state("shelved"), Some(LifecycleState::Archived));
        assert!(host.admit_wake("nobody").is_err());
        assert_eq!(host.state("nobody"), None);
    }

    #[test]
    fn host_lifecycle_undo_and_close_return_worlds_to_asleep() {
        let root = data_root_with(&["w"]);
        let host = HostLifecycle::open(root.path()).unwrap();

        let admission = host.admit_wake("w").unwrap();
        host.undo_wake("w", admission).unwrap();
        let saved = HostFile::load(root.path()).unwrap();
        assert_eq!(saved.worlds["w"].state, LifecycleState::Asleep);
        assert_eq!(saved.worlds["w"].history, [] as [StateChange; 0]);

        host.admit_wake("w").unwrap();
        host.undo_wake("w", Admission::AlreadyLive).unwrap();
        assert_eq!(host.state("w"), Some(LifecycleState::Live));

        host.record_closed("w").unwrap();
        let saved = HostFile::load(root.path()).unwrap();
        let record = &saved.worlds["w"];
        assert_eq!(record.state, LifecycleState::Asleep);
        let actors: Vec<&str> = record.history.iter().map(|c| c.actor.as_str()).collect();
        assert_eq!(actors, [WAKE_ON_CONNECT, IDLE_CLOSE]);

        // Closing a world that is not live records nothing.
        host.record_closed("w").unwrap();
        host.record_closed("nobody").unwrap();
        assert_eq!(HostFile::load(root.path()).unwrap(), saved);
        assert_eq!(host.snapshot(), saved);
    }

    #[test]
    fn operator_wake_mode_refuses_wake_on_connect_and_pins_are_saved() {
        let root = data_root_with(&["kept", "plain"]);
        let host = HostLifecycle::open(root.path()).unwrap();

        // Defaults are not written, so older host files read the same.
        let text = std::fs::read_to_string(HostFile::path_for(root.path())).unwrap();
        assert!(!text.contains("pinned") && !text.contains("wake"), "{text}");
        assert!(!host.pinned("kept"));

        host.set_wake("kept", WakeMode::Operator).unwrap();
        host.set_pinned("kept", true).unwrap();
        assert!(host.pinned("kept"));
        let refused = host.admit_wake("kept").unwrap_err().to_string();
        assert!(refused.contains("operator"), "{refused}");
        assert_eq!(host.state("kept"), Some(LifecycleState::Asleep));
        let refused = host.admit_create("kept").unwrap_err().to_string();
        assert!(refused.contains("operator"), "{refused}");

        let saved = HostFile::load(root.path()).unwrap();
        assert!(saved.worlds["kept"].pinned);
        assert_eq!(saved.worlds["kept"].wake, WakeMode::Operator);
        assert_eq!(saved.worlds["kept"].history, [] as [StateChange; 0]);
        let seen = inspect_world(root.path(), &saved, "kept").unwrap();
        assert!(seen.pinned);
        assert_eq!(seen.wake, WakeMode::Operator);

        // An operator start still brings it live, and a live world is
        // admitted whatever its wake mode.
        let mut file = saved.clone();
        file.transition("kept", LifecycleState::Live, actor(), 1)
            .unwrap();
        file.save(root.path()).unwrap();
        assert_eq!(host.admit_wake("kept").unwrap(), Admission::AlreadyLive);

        assert!(host.set_pinned("nobody", true).is_err());
        assert!(host.set_wake("nobody", WakeMode::Operator).is_err());
        assert_eq!(host.admit_wake("plain").unwrap(), Admission::Woken);
        assert_eq!("operator".parse::<WakeMode>().unwrap(), WakeMode::Operator);
        assert_eq!(WakeMode::Connect.to_string(), "connect");
        assert!("never".parse::<WakeMode>().is_err());
    }

    #[test]
    fn host_lifecycle_adopts_an_operator_edit_saved_by_another_process() {
        let root = data_root_with(&["w"]);
        let host = HostLifecycle::open(root.path()).unwrap();
        assert!(!host.pinned("w"));

        let mut edited = HostFile::load(root.path()).unwrap();
        edited.set_pinned("w", true).unwrap();
        edited.save(root.path()).unwrap();
        assert!(host.pinned("w"));

        // The next transition keeps the edit instead of writing over it.
        host.admit_wake("w").unwrap();
        let saved = HostFile::load(root.path()).unwrap();
        assert!(saved.worlds["w"].pinned);
        assert_eq!(saved.worlds["w"].state, LifecycleState::Live);

        // A replacement that does not load is ignored, and the next save
        // writes the last good file back.
        std::fs::write(HostFile::path_for(root.path()), b"not json").unwrap();
        assert!(host.pinned("w"));
        host.record_closed("w").unwrap();
        let saved = HostFile::load(root.path()).unwrap();
        assert!(saved.worlds["w"].pinned);
        assert_eq!(saved.worlds["w"].state, LifecycleState::Asleep);
    }

    #[test]
    fn host_lifecycle_admit_create_records_new_worlds_and_never_restores() {
        let root = data_root_with(&["shelved"]);
        let host = HostLifecycle::open(root.path()).unwrap();
        host.lock()
            .transition("shelved", LifecycleState::Archived, actor(), 1)
            .unwrap();

        assert_eq!(host.admit_create("fresh").unwrap(), Admission::Created);
        let saved = HostFile::load(root.path()).unwrap();
        let record = &saved.worlds["fresh"];
        assert_eq!(record.state, LifecycleState::Live);
        assert_eq!(record.history[0].actor, ADMIN_CREATE);
        assert_eq!(host.admit_create("fresh").unwrap(), Admission::AlreadyLive);

        assert!(host.admit_create("shelved").is_err());
        assert_eq!(host.state("shelved"), Some(LifecycleState::Archived));
        assert!(host.admit_create("../escape").is_err());
        assert!(host.admit_create(HOST_FILE_NAME).is_err());

        assert_eq!(host.admit_create("doomed").unwrap(), Admission::Created);
        host.undo_wake("doomed", Admission::Created).unwrap();
        assert_eq!(host.state("doomed"), None);
        assert!(
            !HostFile::load(root.path())
                .unwrap()
                .worlds
                .contains_key("doomed")
        );
    }

    #[test]
    fn inspect_world_reads_record_journal_and_modules_without_writing() {
        let root = data_root_with(&["w", "broken"]);
        let created = serde_json::to_string(&crate::world::JournalEntry::Created {
            world_id: "w".to_owned(),
            version: crate::world::JOURNAL_VERSION,
        })
        .unwrap();
        std::fs::write(
            root.path().join("w").join(JOURNAL_FILE),
            format!("{created}\n{{\"torn"),
        )
        .unwrap();
        std::fs::write(
            root.path()
                .join("w/modules")
                .join(format!("{}.wasm", "ab".repeat(32))),
            b"\0asm",
        )
        .unwrap();
        let mut file = HostFile::rebuild(root.path()).unwrap();
        file.transition("w", LifecycleState::Live, actor(), 7)
            .unwrap();

        let seen = inspect_world(root.path(), &file, "w").unwrap();
        assert_eq!(seen.state, LifecycleState::Live);
        assert_eq!(seen.history.len(), 1);
        assert_eq!(seen.history[0].actor, actor());
        let journal = seen.journal.unwrap();
        assert_eq!(journal.entries, 1);
        assert_eq!(journal.version, crate::world::JOURNAL_VERSION);
        assert!(journal.torn_tail);
        assert_eq!(seen.module_count, 1);
        assert_eq!(seen.storage.modules_bytes, 4);
        assert_eq!(seen.dir, root.path().join("w"));

        // An empty journal is reported, not an error.
        let broken = inspect_world(root.path(), &file, "broken").unwrap();
        assert!(broken.journal.is_none());
        assert!(broken.journal_error.unwrap().contains("empty"));

        assert!(inspect_world(root.path(), &file, "nobody").is_err());
        assert!(!HostFile::path_for(root.path()).exists());
    }

    /// Leave `name` as a killed host would: recorded live, nothing held.
    fn record_live_as_after_a_crash(root: &Path, name: &str) {
        let mut file = HostFile::load(root).unwrap();
        file.transition(name, LifecycleState::Live, actor(), 1_700_000_200)
            .unwrap();
        file.save(root).unwrap();
    }

    /// World process killed (#10695): a world's actor dies with its
    /// runtime, mid-stream and with a write in flight, never shut down.
    /// The next host open replays it, every acknowledged entry is there,
    /// nothing half-applied is, and the asleep sibling's directory is not
    /// touched.
    #[test]
    fn a_killed_world_keeps_every_acknowledged_entry() {
        use crate::world::{WorldEventKind, WorldScopes};

        let root = data_root_with(&["alpha", "beta"]);
        let beta_journal = root.path().join("beta").join(JOURNAL_FILE);
        let beta_before = std::fs::read(&beta_journal).unwrap();
        let acknowledged: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();

        let doomed = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (held, ann) = doomed.block_on(async {
            let mut sup = Supervisor::open(root.path()).await.unwrap();
            sup.start("alpha", actor()).await.unwrap();
            let world = sup.live["alpha"].service.world();
            let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
            for line in &acknowledged {
                world.chat(ann.clone(), line.as_str()).await.unwrap();
            }
            let (inflight, token) = (world.clone(), ann.clone());
            tokio::spawn(async move { inflight.chat(token, "in flight").await });
            (sup, ann)
        });
        // Kill: the runtime and every task on it stop where they stand.
        doomed.shutdown_timeout(std::time::Duration::from_secs(10));
        drop(held);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let sup = Supervisor::open(root.path()).await.unwrap();
            assert_eq!(sup.live_names(), vec!["alpha".to_owned()]);
            let recovery = sup.file().worlds["alpha"].last_recovery.clone().unwrap();
            assert_eq!(recovery.error, None);
            let world = sup.live["alpha"].service.world();
            let chats: Vec<String> = world
                .snapshot(ann)
                .await
                .unwrap()
                .events
                .into_iter()
                .filter_map(|event| match event.kind {
                    WorldEventKind::Chat(text) => Some(text),
                    _ => None,
                })
                .collect();
            assert!(
                chats.starts_with(&acknowledged),
                "every acknowledged line survives, in order: {chats:?}"
            );
            assert!(chats.len() <= acknowledged.len() + 1, "{chats:?}");
            if let Some(extra) = chats.get(acknowledged.len()) {
                assert_eq!(
                    extra, "in flight",
                    "an in-flight write lands whole or not at all"
                );
            }
            assert_eq!(sup.file().worlds["beta"].state, LifecycleState::Asleep);
            assert_eq!(std::fs::read(&beta_journal).unwrap(), beta_before);
            drop(sup);
        });
    }

    #[tokio::test]
    async fn a_save_cut_off_before_its_rename_leaves_the_last_host_file() {
        // #10695 power loss: the host died after writing `host.json.tmp`
        // and before renaming it. The saved host file still stands, the
        // half-written temp file is never read, and the next save
        // replaces it.
        let root = data_root_with(&["alpha", "beta"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("alpha", actor()).await.unwrap();
        sup.shutdown().await.unwrap();
        let saved = std::fs::read(HostFile::path_for(root.path())).unwrap();
        let tmp = HostFile::path_for(root.path()).with_extension("json.tmp");
        std::fs::write(&tmp, &saved[..saved.len() / 2]).unwrap();

        let mut sup = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup.live_names(), vec!["alpha".to_owned()]);
        assert_eq!(sup.file().worlds["beta"].state, LifecycleState::Asleep);
        sup.start("beta", actor()).await.unwrap();
        assert!(!tmp.exists());
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.worlds["beta"].state, LifecycleState::Live);
        assert_eq!(file.worlds.len(), 2);
        sup.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_host_file_older_than_the_journal_loses_no_entries() {
        // #10695 power loss: the host file came back older than the
        // journal (a save that never reached the disk). The journal is the
        // only authority on contents, so starting the world replays every
        // fsynced entry, whatever state the old host file records.
        let root = data_root_with(&["alpha"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        let stale = std::fs::read(HostFile::path_for(root.path())).unwrap();
        sup.start("alpha", actor()).await.unwrap();
        sup.live["alpha"]
            .service
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        let want = sup.live_report("alpha").unwrap().clone();
        sup.shutdown().await.unwrap();
        let journal = root.path().join("alpha").join(JOURNAL_FILE);
        let entries = journal_facts(&journal).unwrap().entries;
        std::fs::write(HostFile::path_for(root.path()), &stale).unwrap();

        let mut sup = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup.live_names(), [] as [String; 0]);
        assert_eq!(sup.file().worlds["alpha"].state, LifecycleState::Asleep);
        let got = sup.start("alpha", actor()).await.unwrap();
        assert!(got.journal_entries >= want.journal_entries);
        assert_eq!(
            sup.live["alpha"].service.world().isolation().await.unwrap(),
            Isolation::Unisolated
        );
        sup.shutdown().await.unwrap();
        assert!(journal_facts(&journal).unwrap().entries >= entries);
    }

    #[tokio::test]
    async fn restart_after_a_torn_append_replays_exactly_the_fsynced_prefix() {
        // #10695 crash matrix: the host died mid-append, leaving an
        // incomplete final line after the last fsynced entry.
        let root = data_root_with(&["alpha"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("alpha", actor()).await.unwrap();
        sup.live["alpha"]
            .service
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        sup.shutdown().await.unwrap();
        let journal = root.path().join("alpha").join(JOURNAL_FILE);
        let fsynced = std::fs::read(&journal).unwrap();

        // The oracle: the same prefix replayed in a scratch directory.
        let oracle_root = data_root_with(&["alpha"]);
        std::fs::write(
            oracle_root.path().join("alpha").join(JOURNAL_FILE),
            &fsynced,
        )
        .unwrap();
        let mut oracle = Supervisor::open(oracle_root.path()).await.unwrap();
        oracle.start("alpha", actor()).await.unwrap();
        let want = oracle.live_report("alpha").unwrap().clone();
        let want_isolation = oracle.live["alpha"]
            .service
            .world()
            .isolation()
            .await
            .unwrap();
        oracle.shutdown().await.unwrap();

        let mut torn = fsynced.clone();
        torn.extend_from_slice(br#"{"IsolationChanged":{"isol"#);
        std::fs::write(&journal, &torn).unwrap();
        assert!(journal_facts(&journal).unwrap().torn_tail);

        let sup = Supervisor::open(root.path()).await.unwrap();
        assert_eq!(sup.live_names(), vec!["alpha".to_owned()]);
        let got = sup.live_report("alpha").unwrap();
        assert_eq!(got.sequence, want.sequence);
        assert_eq!(got.journal_entries, want.journal_entries);
        assert_eq!(
            sup.live["alpha"].service.world().isolation().await.unwrap(),
            want_isolation
        );
        assert_eq!(want_isolation, Isolation::Unisolated);
        sup.shutdown().await.unwrap();
        let recovery = HostFile::load(root.path()).unwrap().worlds["alpha"]
            .last_recovery
            .clone()
            .unwrap();
        assert!(recovery.torn_tail);
        assert_eq!(recovery.error, None);
        assert_eq!(recovery.sequence, want.sequence);
        assert_eq!(
            recovery.journal_entries,
            u64::try_from(want.journal_entries).unwrap()
        );
        // No partial application: the torn bytes were never kept as an
        // entry, and every fsynced byte survives.
        let after = std::fs::read(&journal).unwrap();
        assert!(after.starts_with(&fsynced));
        assert_eq!(
            journal_facts(&journal).unwrap().entries,
            journal_facts_of(&fsynced)
        );
    }

    fn journal_facts_of(bytes: &[u8]) -> usize {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join(JOURNAL_FILE);
        std::fs::write(&path, bytes).unwrap();
        journal_facts(&path).unwrap().entries
    }

    #[tokio::test]
    async fn open_refuses_a_journal_version_it_cannot_replay_and_leaves_it_untouched() {
        // #10695: name the version, never rewrite the journal, and never
        // demote the world from the state the host file records.
        let root = data_root_with(&["future", "fine"]);
        let future = crate::world::MAX_JOURNAL_VERSION + 1;
        let created = serde_json::to_string(&crate::world::JournalEntry::Created {
            world_id: "future".to_owned(),
            version: future,
        })
        .unwrap();
        let journal = root.path().join("future").join(JOURNAL_FILE);
        std::fs::write(&journal, format!("{created}\n")).unwrap();
        let before = std::fs::read(&journal).unwrap();
        HostFile::rebuild(root.path())
            .unwrap()
            .save(root.path())
            .unwrap();
        record_live_as_after_a_crash(root.path(), "future");

        let error = Supervisor::open(root.path()).await.unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("future"), "{message}");
        assert!(message.contains(&future.to_string()), "{message}");
        assert_eq!(std::fs::read(&journal).unwrap(), before);
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.worlds["future"].state, LifecycleState::Live);
        assert_eq!(file.worlds["fine"].state, LifecycleState::Asleep);
        // The failed replay is recorded for `id host inspect`.
        let recovery = file.worlds["future"].last_recovery.clone().unwrap();
        let recorded = recovery.error.unwrap();
        assert!(recorded.contains(&future.to_string()), "{recorded}");
        assert_eq!(file.worlds["fine"].last_recovery, None);
        let seen = inspect_world(root.path(), &file, "future").unwrap();
        assert!(seen.last_recovery.is_some());
        let defaults = WorldLimits::default();
        assert_eq!(
            seen.ceilings.participants,
            u64::try_from(defaults.participants).unwrap()
        );
        assert_eq!(seen.ceilings.checkpoint_every, defaults.checkpoint_every);
        assert_eq!(
            seen.ceilings.total_memory_bytes,
            RuntimeLimits::default().total_memory_bytes
        );
    }

    #[test]
    fn host_lifecycle_reopens_what_it_saved() {
        let root = data_root_with(&["w"]);
        HostLifecycle::open(root.path())
            .unwrap()
            .admit_wake("w")
            .unwrap();
        let again = HostLifecycle::open(root.path()).unwrap();
        assert_eq!(again.state("w"), Some(LifecycleState::Live));
        assert_eq!(again.root(), root.path());
    }

    #[test]
    fn read_or_rebuild_and_census_of_write_nothing() {
        let root = data_root_with(&["a", "b"]);
        let file = HostFile::read_or_rebuild(root.path()).unwrap();
        assert!(!HostFile::path_for(root.path()).exists());
        assert_eq!(file.worlds.len(), 2);
        let census = census_of(root.path(), &file);
        assert_eq!(census.asleep.worlds, 2);
        assert_eq!(census.live.worlds, 0);
        assert!(!HostFile::path_for(root.path()).exists());

        // A saved file is read as saved, not rebuilt.
        let mut saved = HostFile::rebuild(root.path()).unwrap();
        saved
            .transition("b", LifecycleState::Archived, actor(), 1_700_000_000)
            .unwrap();
        saved.save(root.path()).unwrap();
        assert_eq!(HostFile::read_or_rebuild(root.path()).unwrap(), saved);
    }

    #[tokio::test]
    async fn census_reports_per_state_counts_and_bytes() {
        let root = data_root_with(&["l1", "s1", "v1"]);
        // Give s1 a module blob and v1 an extra sidecar file.
        std::fs::write(
            root.path()
                .join("s1")
                .join("modules")
                .join(format!("{}.wasm", "ab".repeat(32))),
            vec![7u8; 64],
        )
        .unwrap();
        std::fs::write(root.path().join("v1").join("notes.txt"), b"hello").unwrap();

        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("l1", actor()).await.unwrap();
        sup.archive("v1", actor()).unwrap();

        let census = sup.census();
        assert_eq!(census.live.worlds, 1);
        assert_eq!(census.asleep.worlds, 1);
        assert_eq!(census.archived.worlds, 1);
        let asleep_row = census.worlds.iter().find(|w| w.name == "s1").unwrap();
        assert_eq!(asleep_row.modules_bytes, 64);
        let archived_row = census.worlds.iter().find(|w| w.name == "v1").unwrap();
        assert_eq!(archived_row.other_bytes, 5);
        assert!(!archived_row.missing);
    }

    #[test]
    fn host_file_proptest_roundtrip() {
        use proptest::prelude::*;

        fn state_strategy() -> impl Strategy<Value = LifecycleState> {
            prop_oneof![
                Just(LifecycleState::Live),
                Just(LifecycleState::Asleep),
                Just(LifecycleState::Archived),
            ]
        }

        let name = "[a-z][a-z0-9-]{0,15}";
        let change =
            (0u64..u32::MAX as u64, ".+", state_strategy()).prop_map(|(at_secs, actor, prior)| {
                StateChange {
                    at_secs,
                    actor,
                    prior,
                }
            });
        let record = (
            state_strategy(),
            proptest::collection::vec(change, 0..4),
            proptest::bool::ANY,
            proptest::bool::ANY,
            proptest::option::of((0u64..u32::MAX as u64, proptest::bool::ANY, 0u64..64)),
        )
            .prop_map(|(state, history, pinned, operator, recovery)| WorldRecord {
                state,
                history,
                pinned,
                wake: if operator {
                    WakeMode::Operator
                } else {
                    WakeMode::Connect
                },
                last_recovery: recovery.map(|(at_secs, torn_tail, entries)| RecoveryRecord {
                    at_secs,
                    torn_tail,
                    sequence: entries,
                    journal_entries: entries,
                    replayed_inputs: 0,
                    program_error: None,
                    error: (entries % 2 == 1).then(|| "replay failed".to_owned()),
                }),
                snapshots: None,
            });
        let file = (
            proptest::collection::hash_map(name, record, 0..5),
            proptest::collection::vec(0u8..8, 0..4),
        )
            .prop_map(|(worlds, policy)| HostFile {
                version: HOST_FILE_VERSION,
                data_root: PathBuf::from("/tmp/host-proptest-root"),
                worlds: worlds.into_iter().collect(),
                snapshots: policy.first().map(|keep| SnapshotPolicy {
                    every_secs: 3_600,
                    keep: u32::from(*keep) + 1,
                    max_age_secs: (keep % 2 == 0).then_some(86_400),
                }),
                snapshot_dir: None,
            });

        proptest::test_runner::TestRunner::default()
            .run(&file, |file| {
                let bytes = serde_json::to_vec(&file).unwrap();
                let back: HostFile = serde_json::from_slice(&bytes).unwrap();
                prop_assert_eq!(file, back);
                Ok(())
            })
            .unwrap();
    }

    fn hourly(keep: u32) -> SnapshotPolicy {
        SnapshotPolicy {
            every_secs: 3_600,
            keep,
            max_age_secs: None,
        }
    }

    async fn snapshot_host(names: &[&str]) -> (TempDir, Supervisor) {
        let root = data_root_with(names);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.set_signing_key(SigningKey::from_bytes(&[9; 32]));
        (root, sup)
    }

    #[tokio::test]
    async fn scheduled_snapshots_follow_cadence_and_retention() {
        use crate::world::WorldScopes;

        let (root, mut sup) = snapshot_host(&["alpha", "beta"]).await;
        sup.start("alpha", actor()).await.unwrap();
        // Off by default: nothing is taken.
        assert!(sup.run_due_snapshots(1_000).await.is_empty());

        sup.set_snapshot_policy(None, Some(hourly(2))).unwrap();
        let first = sup.run_due_snapshots(1_000).await;
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        // Only live worlds are snapshotted.
        assert_eq!(first.taken.len(), 1);
        assert_eq!(first.taken[0].world, "alpha");
        let snapshot_root = sup.file().snapshot_root(root.path());
        assert_eq!(snapshot_root, root.path().join(SNAPSHOT_DIR_NAME));
        assert_eq!(list_sets(&snapshot_root, "alpha").unwrap().len(), 1);

        // Not due again inside the cadence.
        assert!(sup.run_due_snapshots(1_000 + 3_599).await.is_empty());

        let world = sup.live["alpha"].service.world();
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        for (step, at) in [(1, 5_000), (2, 9_000), (3, 13_000)] {
            world
                .chat(ann.clone(), format!("change {step}"))
                .await
                .unwrap();
            let run = sup.run_due_snapshots(at).await;
            assert!(run.failed.is_empty(), "{:?}", run.failed);
            assert_eq!(run.taken.len(), 1);
        }
        let sets = list_sets(&snapshot_root, "alpha").unwrap();
        // Retention keeps the newest two whole sets.
        assert_eq!(
            sets.iter().map(|set| set.at_secs).collect::<Vec<_>>(),
            vec![9_000, 13_000]
        );
        assert!(sets[0].entries < sets[1].entries);

        // An unchanged journal: the fresh set replaces the newest one.
        let run = sup.run_due_snapshots(17_000).await;
        assert_eq!(run.taken.len(), 1);
        assert_eq!(run.removed.len(), 1);
        assert_eq!(run.removed[0].at_secs, 13_000);
        let sets = list_sets(&snapshot_root, "alpha").unwrap();
        assert_eq!(
            sets.iter().map(|set| set.at_secs).collect::<Vec<_>>(),
            vec![9_000, 17_000]
        );

        // Every set restores: the newest one into a scratch root.
        let source = *SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes();
        let scratch = TempDir::new().unwrap();
        let manifest = import_world_from_file(&sets[1].path, &scratch.path().join("alpha"), source)
            .await
            .unwrap();
        assert_eq!(manifest.entries, sets[1].entries);

        // The census counts the sets against the live state.
        let census = sup.census();
        let alpha = census
            .worlds
            .iter()
            .find(|row| row.name == "alpha")
            .unwrap();
        assert_eq!(alpha.snapshot_sets, 2);
        let bytes: u64 = sets.iter().map(|set| set.bytes).sum();
        assert_eq!(alpha.snapshot_bytes, bytes);
        assert_eq!(census.live.snapshot_bytes, bytes);
        assert_eq!(census.asleep.snapshot_bytes, 0);

        // The journal itself is never touched by pruning.
        let journal = std::fs::read(root.path().join("alpha").join(JOURNAL_FILE)).unwrap();
        assert!(!journal.is_empty());
        sup.stop_all(actor()).await.unwrap();
    }

    #[tokio::test]
    async fn a_world_policy_overrides_the_host_default() {
        let (root, mut sup) = snapshot_host(&["alpha", "beta"]).await;
        sup.start("alpha", actor()).await.unwrap();
        sup.start("beta", actor()).await.unwrap();
        sup.set_snapshot_policy(None, Some(hourly(3))).unwrap();
        sup.set_snapshot_policy(Some("beta"), Some(SnapshotPolicy::OFF))
            .unwrap();
        let run = sup.run_due_snapshots(100).await;
        assert_eq!(
            run.taken
                .iter()
                .map(|set| set.world.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"]
        );
        // The override survives a reload of the host file.
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.snapshot_policy("beta"), SnapshotPolicy::OFF);
        assert_eq!(file.snapshot_policy("alpha"), hourly(3));
        // Clearing the override makes beta follow the default again.
        sup.set_snapshot_policy(Some("beta"), None).unwrap();
        let run = sup.run_due_snapshots(200).await;
        assert_eq!(
            run.taken
                .iter()
                .map(|set| set.world.as_str())
                .collect::<Vec<_>>(),
            vec!["beta"]
        );
        // Bad policies and unknown worlds are refused and change nothing.
        assert!(sup.set_snapshot_policy(None, Some(hourly(0))).is_err());
        assert!(
            sup.set_snapshot_policy(Some("gamma"), Some(hourly(1)))
                .is_err()
        );
        assert_eq!(
            HostFile::load(root.path()).unwrap().snapshots,
            Some(hourly(3))
        );
        sup.stop_all(actor()).await.unwrap();
    }

    #[tokio::test]
    async fn snapshots_go_to_the_named_directory_and_need_a_host_key() {
        let root = data_root_with(&["alpha"]);
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("alpha", actor()).await.unwrap();
        sup.set_snapshot_policy(None, Some(hourly(1))).unwrap();
        // No signing key: nothing is taken, nothing fails.
        assert!(sup.run_due_snapshots(100).await.is_empty());

        sup.stop_all(actor()).await.unwrap();
        sup.shutdown().await.unwrap();
        let elsewhere = TempDir::new().unwrap();
        let mut file = HostFile::load(root.path()).unwrap();
        assert!(
            file.set_snapshot_dir(Some(PathBuf::from("relative")))
                .is_err()
        );
        file.set_snapshot_dir(Some(elsewhere.path().to_path_buf()))
            .unwrap();
        file.save(root.path()).unwrap();

        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.set_signing_key(SigningKey::from_bytes(&[9; 32]));
        sup.start("alpha", actor()).await.unwrap();
        let run = sup.run_due_snapshots(100).await;
        assert_eq!(run.taken.len(), 1);
        assert!(run.taken[0].path.starts_with(elsewhere.path()));
        assert!(!root.path().join(SNAPSHOT_DIR_NAME).exists());
        sup.stop_all(actor()).await.unwrap();
    }

    #[test]
    fn the_snapshot_directory_name_is_not_a_world_name() {
        assert!(validate_name(SNAPSHOT_DIR_NAME).is_err());
    }
}
