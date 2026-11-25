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
use serde::{Deserialize, Serialize};

use crate::world::{WorldHandle, WorldLimits};
use crate::world_doctor::{JournalFacts, journal_facts};
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
}

impl HostFile {
    /// Empty host file for `root`.
    pub fn empty(root: &Path) -> Self {
        Self {
            version: HOST_FILE_VERSION,
            data_root: root.to_path_buf(),
            worlds: BTreeMap::new(),
        }
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
                    history: Vec::new(),
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
        self.worlds.push(row);
    }
}

/// Per-state counts + bytes over the worlds `file` records under `root`,
/// without opening any world for serving. Missing directories census as
/// zero-byte rows, never as errors.
#[must_use]
pub fn census_of(root: &Path, file: &HostFile) -> Census {
    let mut census = Census::default();
    for (name, record) in &file.worlds {
        let dir = root.join(name);
        let mut row = WorldCensus {
            name: name.clone(),
            state: record.state,
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
        })
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
    /// Refuses an archived world (restore it first), a world the host
    /// file does not know, and a failed save (nothing is recorded then).
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
            },
        );
        next.save(&self.root)?;
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
            return file.save(&self.root);
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
        file.save(&self.root)
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
        next.save(&self.root)?;
        *file = next;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HostFile> {
        self.file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    /// The journal's entry count, version and torn tail, when it parses.
    pub journal: Option<JournalFacts>,
    /// Why the journal could not be read, when it could not.
    pub journal_error: Option<String>,
    /// Module files under `modules/`.
    pub module_count: u64,
    /// The world's storage row, as `id host list` shows it.
    pub storage: WorldCensus,
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
        journal,
        journal_error,
        module_count,
        storage,
    })
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

/// The supervisor: one data root, the host file beside it, and the held
/// live worlds. Created by [`Supervisor::open`], which replays every
/// world the host file records live.
#[derive(Debug)]
pub struct Supervisor {
    root: PathBuf,
    file: HostFile,
    live: HashMap<String, LiveWorld>,
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
        };
        let live_names: Vec<String> = supervisor
            .file
            .worlds
            .iter()
            .filter(|(_, record)| record.state == LifecycleState::Live)
            .map(|(name, _)| name.clone())
            .collect();
        let mut failures = Vec::new();
        for name in live_names {
            if let Err(error) = supervisor.replay_live(&name).await {
                failures.push(format!("{name}: {error:#}"));
            }
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
        validate_name(name)?;
        if self.file.worlds.contains_key(name) {
            bail!("world {name:?} is already in the host file");
        }
        let dir = self.root.join(name);
        if !dir.is_dir() {
            bail!("no world directory at {}", dir.display());
        }
        if !dir.join("journal.jsonl").is_file() {
            bail!(
                "{} has no journal.jsonl: not a world directory",
                dir.display()
            );
        }
        self.file.worlds.insert(
            name.to_owned(),
            WorldRecord {
                state: LifecycleState::Asleep,
                history: vec![StateChange {
                    at_secs: now_secs(),
                    actor: actor.to_owned(),
                    prior: LifecycleState::Asleep,
                }],
            },
        );
        self.file.save(&self.root)?;
        Ok(())
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
    async fn replay_live(&mut self, name: &str) -> Result<OpenReport> {
        let dir = self.root.join(name);
        let world_id = name.to_owned();
        let (service, report) = open_world(
            &dir,
            &world_id,
            WorldLimits::default(),
            RuntimeLimits::default(),
            |handle: WorldHandle| WorldService::new(handle, None),
        )
        .await
        .with_context(|| format!("replaying world {name:?}"))?;
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
        assert!(sup.live_names().is_empty());

        // Append a durable entry to alpha, then stop: the journal now
        // proves replay on the next open.
        sup.start("alpha", actor()).await.unwrap();
        let world = sup.live.get("alpha").unwrap().service.world();
        world
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        let before = sup.live_report("alpha").unwrap().journal_entries;
        assert_eq!(before, 1);
        sup.stop_all(actor()).await.unwrap();
        assert!(sup.live_names().is_empty());
        drop(sup);

        // Clean restart: nothing is live, so nothing replays, and the
        // journal survives byte-identical.
        let journal_before =
            std::fs::read(root.path().join("alpha").join("journal.jsonl")).unwrap();
        let sup2 = Supervisor::open(root.path()).await.unwrap();
        assert!(sup2.live_names().is_empty());
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
        let replayed = std::fs::read(root.path().join("alpha").join("journal.jsonl")).unwrap();
        assert!(replayed.starts_with(&journal_before));
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
        assert!(!journal_before.is_empty());

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
        assert_eq!(sup2.file(), &file_before);
    }

    #[tokio::test]
    async fn delete_and_rebuild_loses_no_world_or_journal_data() {
        let root = data_root_with(&["a", "b"]);
        let journal_a = std::fs::read(root.path().join("a").join("journal.jsonl")).unwrap();
        let mut sup = Supervisor::open(root.path()).await.unwrap();
        sup.start("a", actor()).await.unwrap();
        let world = sup.live.get("a").unwrap().service.world();
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
        assert!(saved.worlds["w"].history.is_empty());

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
        std::fs::write(root.path().join("w/modules/aa.wasm"), b"\0asm").unwrap();
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
        let record = (state_strategy(), proptest::collection::vec(change, 0..4))
            .prop_map(|(state, history)| WorldRecord { state, history });
        let file = (
            proptest::collection::hash_map(name, record, 0..5),
            proptest::collection::vec(0u8..8, 0..4),
        )
            .prop_map(|(worlds, _)| HostFile {
                version: HOST_FILE_VERSION,
                data_root: PathBuf::from("/tmp/host-proptest-root"),
                worlds: worlds.into_iter().collect(),
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
}
