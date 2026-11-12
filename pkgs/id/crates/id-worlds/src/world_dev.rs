//! `id world dev` — the local, offline authoring loop for Roc world apps
//! (F12.15, issue #5541; design `doc/2026-11-01_design_id558.md`).
//!
//! Phase 1 (this module): the foundational slice. One-shot compile of an
//! app directory with the existing bounded compiler engine
//! ([`crate::world_compile`]), launched into a disposable in-memory
//! scratch world built from the existing local actor seam
//! ([`WorldCore`] + [`WorldHandle`] with a sandbox guest). No host is
//! contacted and no remote client is constructed: the scratch world is
//! isolated from every named world and is discarded when the session
//! ends, so a fresh launch always starts from the compiled program's
//! initial state.
//!
//! Phase 3 (this module): fixture replay as a promotion gate. Fixtures
//! given to a dev session are replayed against the compiled candidate
//! before it is launched; a candidate whose replayed records mismatch
//! the fixture's recorded expectation is not promoted (not launched).
//! The gate is the design's dev-only cross-module-hash regression mode:
//! unlike ordinary `replay`/`verify`, it does not refuse a fixture
//! recorded under a different module — it replays it anyway and
//! reports the cross-module fact on the gate result (invariant 5).
//!
//! Phase 4 (this module): deterministic fuzzing against declared
//! invariants. A bounded, versioned declaration file names a
//! generated input alphabet and the invariants a candidate must
//! hold; a seeded generator (never host entropy) produces case
//! sequences over that alphabet, each applied to a fresh scratch
//! world, and the campaign stops at the first failure with a
//! reproducible counterexample. Fuzzing is a separate, opt-in check:
//! it never runs on the edit-to-running path.
//!
//! Phase 2 (this module): watch, rebuild, and transactional
//! hot-swap. Declared inputs carry a source snapshot identity
//! ([`DevSources::snapshot_id`], invariant 2); a [`Debouncer`]
//! coalesces change bursts and a bounded rescan produces the next
//! candidate snapshot. Promotion into the live session is
//! transactional ([`DevSession::promote`]): stale candidates are
//! discarded, the Phase 3 fixture gates run first, the state
//! transition goes through F12.9's probe/migrate/commit
//! ([`crate::world::upgrade_program`], invariant 4), and the
//! candidate must replay the session's input history to the
//! migrated records before it becomes active — every failure
//! leaves the last-good program running, with the diagnostic in
//! the session's last-good record (invariant 3).
//!
//! Later phases (latency benchmark + authoring docs) build on
//! [`DevSession`] and [`run_dev_watch`].

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::sandbox::{Sandbox, SandboxLimits};
use crate::world::{JoinCapability, Records, WorldCore, WorldHandle, WorldLimits, WorldScopes};
use crate::world_compile::{COMPILE_TIMEOUT, CompileSpec, Compiler, validate_spec};
use crate::world_fixture::{Fixture, Verdict};

/// World id of the disposable scratch world. It exists only in this
/// process's memory and is never served, journaled, or replicated.
pub const DEV_WORLD_ID: &str = "dev-scratch";

/// Per-file read cap while discovering sources. This is the compiler's
/// total-request bound, so any file larger than this could never pass
/// [`validate_spec`]; discovery refuses it before reading it whole.
const MAX_DISCOVERY_FILE_BYTES: u64 = 256 * 1024;

/// The Roc sources of one app directory, in compile order: `main.roc`
/// first, then the remaining `.roc` files by name. This is the declared
/// input set the dev loop watches in later phases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevSources {
    /// The app directory the sources were read from.
    pub dir: PathBuf,
    /// `(file name, UTF-8 content)` pairs, `main.roc` first.
    pub files: Vec<(String, String)>,
}

impl DevSources {
    /// The bounded compile request for these sources.
    #[must_use]
    pub fn spec(&self, seed: u64) -> CompileSpec {
        CompileSpec {
            files: self.files.clone(),
            seed,
            native: false,
        }
    }

    /// The text of `main.roc` (always present and always first).
    #[must_use]
    pub fn main_source(&self) -> &str {
        &self.files[0].1
    }
}

/// Discover the declared sources of the app in `dir`.
///
/// Only `.roc` files directly inside `dir` are inputs, matching the
/// compiler's flat file model. Reads are size-capped per file and the
/// result is validated with the compiler's own bounds, so discovery
/// fails with the same errors a compile would.
///
/// # Errors
///
/// `dir` is not a directory, `main.roc` is missing, a source cannot be
/// read as UTF-8 within the read cap, or the set violates the
/// compiler's bounds (file count, per-file and total size, safe names).
pub fn discover_sources(dir: &Path) -> Result<DevSources> {
    ensure!(dir.is_dir(), "{} is not an app directory", dir.display());
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "roc")
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    ensure!(
        names.iter().any(|name| name == "main.roc"),
        "{} has no main.roc; a world app directory starts at main.roc",
        dir.display()
    );
    // Compile order: main.roc first, the rest in name order.
    names.sort_by(|a, b| {
        let rank = |name: &String| (name != "main.roc", name.clone());
        rank(a).cmp(&rank(b))
    });
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let path = dir.join(&name);
        let size = std::fs::metadata(&path)
            .with_context(|| format!("stat {}", path.display()))?
            .len();
        ensure!(
            size <= MAX_DISCOVERY_FILE_BYTES,
            "{name} exceeds the compiler's input bounds ({} bytes)",
            MAX_DISCOVERY_FILE_BYTES
        );
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read source {}", path.display()))?;
        files.push((name, content));
    }
    let sources = DevSources {
        dir: dir.to_path_buf(),
        files,
    };
    validate_spec(&sources.spec(0))?;
    Ok(sources)
}

/// Extract the platform path an app declares in its `main.roc` header
/// (`platform "..."`), if any. The path is returned exactly as written;
/// resolving it is [`resolve_platform_dir`]'s job.
#[must_use]
pub fn declared_platform_path(main_source: &str) -> Option<String> {
    // Only the header can declare the platform; stop at the first
    // blank line so a `platform "..."` string in app code is ignored.
    let header = main_source.split("\n\n").next().unwrap_or(main_source);
    let bytes = header.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        // Find the next `platform` keyword on a word boundary.
        let Some(found) = header[index..].find("platform") else {
            return None;
        };
        let start = index + found;
        let before_ok =
            start == 0 || !bytes[start - 1].is_ascii_alphanumeric() && bytes[start - 1] != b'_';
        let after = start + "platform".len();
        let after_ok =
            after >= bytes.len() || !bytes[after].is_ascii_alphanumeric() && bytes[after] != b'_';
        index = after;
        if !before_ok || !after_ok {
            continue;
        }
        let rest = header[after..].trim_start();
        if let Some(quoted) = rest.strip_prefix('"')
            && let Some(end) = quoted.find('"')
        {
            return Some(quoted[..end].to_owned());
        }
    }
    None
}

/// Resolve the world platform directory for an app.
///
/// An explicit `platform` argument wins (a `platform.roc` file path is
/// accepted and its parent used). Otherwise the path declared in
/// `main.roc` is resolved against the app directory. The result is not
/// validated here: [`Compiler::new`] checks it looks like a platform.
///
/// # Errors
///
/// Neither an explicit platform nor a declared platform path exists.
pub fn resolve_platform_dir(
    dir: &Path,
    explicit: Option<&Path>,
    main_source: &str,
) -> Result<PathBuf> {
    if let Some(platform) = explicit {
        if platform.is_file() {
            return Ok(platform
                .parent()
                .context("platform path has no parent directory")?
                .to_path_buf());
        }
        return Ok(platform.to_path_buf());
    }
    let declared = declared_platform_path(main_source).with_context(|| {
        format!(
            "{} declares no platform path in main.roc; pass --platform",
            dir.display()
        )
    })?;
    let resolved = dir.join(declared);
    // A declared path naming a `.roc` file (the usual
    // `../roc-world/platform.roc`) means its parent directory; the
    // file need not exist yet for the directory to be identified —
    // `Compiler::new` reports a platform that is not built.
    if resolved.is_file() || resolved.extension().is_some_and(|ext| ext == "roc") {
        return Ok(resolved
            .parent()
            .context("declared platform path has no parent directory")?
            .to_path_buf());
    }
    Ok(resolved)
}

/// The platform's module names, mirroring the compiler's own discovery
/// (every `.roc` stem in the platform directory but `platform`). A
/// local dev session grants the scratch world every module its
/// platform provides; hosted worlds keep their capability ceilings.
fn platform_module_names(platform_dir: &Path) -> Result<Vec<String>> {
    let mut modules = Vec::new();
    for entry in std::fs::read_dir(platform_dir).context("read platform directory")? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "roc")
            && let Some(stem) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| *stem != "platform")
        {
            modules.push(stem.to_owned());
        }
    }
    modules.sort();
    Ok(modules)
}

/// A running disposable scratch world: an in-memory [`WorldCore`] whose
/// guest is the compiled module, driven through its actor handle.
///
/// Dropping the session ends the actor; nothing is journaled, served,
/// or left behind (invariant 1 of the design: no remote side effects,
/// scratch state discarded on exit).
pub struct DevScratchWorld {
    handle: WorldHandle,
    capability: JoinCapability,
}

impl DevScratchWorld {
    /// Compile `module` in the sandbox and launch it as the scratch
    /// world's guest program, initialized with `seed`.
    ///
    /// # Errors
    ///
    /// The module fails sandbox compilation/instantiation or the
    /// scratch world's guest capability cannot be issued.
    pub async fn launch(module: &[u8], seed: u64) -> Result<Self> {
        let runner = Sandbox::compile(module, SandboxLimits::default())?;
        Self::launch_compiled(&runner, seed).await
    }

    /// Launch a scratch world from an already-compiled [`Sandbox`],
    /// instantiating a fresh isolated guest for this world. Callers
    /// that launch many worlds from one module (the fuzz campaign)
    /// compile once with [`Sandbox::compile`] and instantiate per
    /// world through here, instead of recompiling per launch.
    ///
    /// # Errors
    ///
    /// The compiled module fails instantiation or the scratch
    /// world's guest capability cannot be issued.
    pub async fn launch_compiled(runner: &Sandbox, seed: u64) -> Result<Self> {
        let guest = runner.instantiate(seed)?;
        let core = WorldCore::new(DEV_WORLD_ID, WorldLimits::default())?;
        let handle = WorldHandle::spawn_with_program(core, Box::new(guest));
        let (_, capability) = handle.issue("dev", WorldScopes::GUEST).await?;
        Ok(Self { handle, capability })
    }

    /// The scratch world's current view, as its guest renders it.
    ///
    /// # Errors
    ///
    /// The actor refuses or the guest call fails.
    pub async fn view(&self) -> Result<Option<String>> {
        self.handle.view(self.capability.clone()).await
    }

    /// Send one input to the scratch world as the dev guest.
    ///
    /// # Errors
    ///
    /// The actor refuses or the guest call fails.
    pub async fn input(&self, input: &[u8]) -> Result<()> {
        self.handle
            .input(self.capability.clone(), input.to_vec())
            .await?;
        Ok(())
    }

    /// Whether the scratch world's program is still healthy after
    /// all processing triggered so far — including the post-input
    /// `after_change` pass (`plaza_wants` and the requests it
    /// issues), which runs after the input reply is sent. The query
    /// is answered in actor order, after that pass completes; see
    /// [`WorldHandle::program_healthy`].
    ///
    /// # Errors
    ///
    /// The world actor is closed or drops the response.
    pub async fn program_healthy(&self) -> Result<bool> {
        self.handle.program_healthy().await
    }

    /// The records sequence the scratch world has reached.
    #[must_use]
    pub fn records_sequence(&self) -> u64 {
        self.handle.records().0
    }

    /// The scratch world's current records, as its guest wrote them.
    #[must_use]
    pub fn records(&self) -> crate::world::Records {
        (*self.handle.records().1).clone()
    }
}

/// The outcome of replaying one fixture against one compiled dev
/// candidate (Phase 3 promotion gate). See [`fixture_gate`].
#[derive(Clone, Debug, Serialize)]
pub struct FixtureGate {
    /// The fixture's label (the path it was read from, as given).
    pub fixture: String,
    /// The world the fixture's history belongs to.
    pub world_id: String,
    /// Module hash the fixture was recorded under, if it records one.
    pub recorded_module_hash: Option<String>,
    /// Module hash of the candidate the fixture was replayed against.
    pub candidate_module_hash: String,
    /// True when the fixture was recorded under a different module
    /// than the candidate. Dev replays it anyway (the cross-module
    /// regression mode); ordinary `replay`/`verify` refuse this.
    pub cross_module: bool,
    /// True when the fixture's history starts at sequence one.
    pub complete: bool,
    /// `match`, `mismatch`, or `not_compared` (see `verdict_reason`).
    pub verdict: String,
    /// Why nothing was compared, when `verdict` is `not_compared`:
    /// a partial history whose final epoch is not covered, or a
    /// fixture that records no expectation. A not-compared fixture
    /// never silently passes as a match.
    pub verdict_reason: Option<String>,
    /// True when this gate refuses the candidate's promotion: the
    /// replayed records mismatch the recorded expectation.
    pub blocks_promotion: bool,
}

/// Replay `fixture` against the compiled candidate `module` and
/// decide whether the candidate may be promoted (Phase 3 gate).
///
/// This is the dev-only cross-module regression mode of invariant 5:
/// [`crate::world_fixture::check_module`] is deliberately not called,
/// so a fixture recorded under an earlier module hash still replays;
/// the gate result reports that fact as `cross_module`. A partial
/// fixture replays its retained window; its records are compared only
/// when the window still covers the final program epoch, and the
/// verdict says `not_compared` (with the reason) when they are not.
///
/// # Errors
///
/// The fixture records no active module, holds an event kind that
/// cannot be replayed, or the candidate fails in the sandbox. The
/// error names the fixture, so an unsupported fixture is a clear
/// diagnostic, never a silent pass.
pub fn fixture_gate(
    module: &[u8],
    seed: u64,
    label: &str,
    fixture: &Fixture,
) -> Result<FixtureGate> {
    let candidate_module_hash = crate::world_session::module_hash(module);
    let recorded_module_hash = fixture.header.module_hash.clone();
    let cross_module = recorded_module_hash.as_deref() != Some(candidate_module_hash.as_str());
    let runner = Sandbox::compile(module, SandboxLimits::default())
        .with_context(|| format!("fixture {label}: compile the candidate"))?;
    let replayed = crate::world_fixture::replay(&runner, seed, fixture, true)
        .with_context(|| format!("fixture {label}"))?;
    let (verdict, verdict_reason) = match &replayed.verdict {
        Verdict::Match => ("match", None),
        Verdict::Mismatch => ("mismatch", None),
        Verdict::NotCompared(reason) => ("not_compared", Some((*reason).to_owned())),
    };
    Ok(FixtureGate {
        fixture: label.to_owned(),
        world_id: fixture.header.world_id.clone(),
        recorded_module_hash,
        candidate_module_hash,
        cross_module,
        complete: fixture.header.complete,
        verdict: verdict.to_owned(),
        verdict_reason,
        blocks_promotion: matches!(replayed.verdict, Verdict::Mismatch),
    })
}

/// One input applied during a one-shot dev session, with the view the
/// guest rendered immediately after it.
#[derive(Clone, Debug, Serialize)]
pub struct DevStep {
    /// The input text that was sent.
    pub input: String,
    /// The view after the input (`null` when the guest renders none).
    pub view: Option<String>,
}

/// The outcome of a one-shot `id world dev` session (Phase 1).
#[derive(Clone, Debug, Serialize)]
pub struct DevReport {
    /// The scratch world's id (always [`DEV_WORLD_ID`]).
    pub world_id: &'static str,
    /// The app directory that was compiled.
    pub dir: PathBuf,
    /// Source file names in compile order.
    pub source_files: Vec<String>,
    /// Seed the guest was initialized with.
    pub seed: u64,
    /// Size of the compiled Wasm module, in bytes.
    pub module_bytes: usize,
    /// Compiler diagnostics for the successful build (may be empty).
    pub compiler_diagnostics: String,
    /// How the module entered the world (source-build provenance).
    pub provenance: serde_json::Value,
    /// The view before any input was sent (`null` when a fixture
    /// gate refused promotion and nothing was launched).
    pub initial_view: Option<String>,
    /// Inputs applied, in order, each with its resulting view.
    pub steps: Vec<DevStep>,
    /// Records sequence reached by the end of the session.
    pub records_sequence: u64,
    /// Fixture promotion gates run against the candidate before
    /// launch (Phase 3), in the order the fixtures were given.
    pub fixture_gates: Vec<FixtureGate>,
    /// True when the candidate was launched. False only when a
    /// fixture gate blocked promotion: nothing was launched, no
    /// input was applied, and the gate results say why.
    pub promoted: bool,
}

/// Run one dev session: discover, compile, run the fixture
/// promotion gates (Phase 3), launch, apply `inputs`, and report.
/// The scratch world is dropped (discarded) before this returns.
///
/// When a fixture gate blocks promotion the candidate is not
/// launched: the report's `promoted` is false, its gate results name
/// the mismatching fixture, and no input is applied. In a one-shot
/// session there is no last-good version to retain — nothing is
/// running, and no hosted world is ever touched.
///
/// # Errors
///
/// Discovery, platform/compiler setup, compilation, a fixture gate
/// error (an unsupported or module-less fixture), or a guest call
/// fails; the error names the failing stage.
pub async fn run_dev_once(
    dir: &Path,
    platform: Option<&Path>,
    roc: Option<&str>,
    seed: u64,
    inputs: &[String],
    fixtures: &[(String, Fixture)],
) -> Result<DevReport> {
    let sources = discover_sources(dir)?;
    let platform_dir = resolve_platform_dir(dir, platform, sources.main_source())?;
    let roc_bin = match roc {
        Some(roc) => roc.to_owned(),
        None => crate::world_compile::locate_roc()?,
    };
    let compiler = Compiler::new(platform_dir.clone(), roc_bin)?;
    let granted = platform_module_names(&platform_dir)?;
    let (module, diagnostics, provenance) =
        crate::world_compile::compile(&sources.spec(seed), &compiler, &granted, COMPILE_TIMEOUT)
            .await?;
    let mut fixture_gates = Vec::with_capacity(fixtures.len());
    for (label, fixture) in fixtures {
        fixture_gates.push(fixture_gate(&module, seed, label, fixture)?);
    }
    if fixture_gates.iter().any(|gate| gate.blocks_promotion) {
        return Ok(DevReport {
            world_id: DEV_WORLD_ID,
            dir: dir.to_path_buf(),
            source_files: sources.files.iter().map(|(name, _)| name.clone()).collect(),
            seed,
            module_bytes: module.len(),
            compiler_diagnostics: diagnostics,
            provenance: serde_json::to_value(&provenance)?,
            initial_view: None,
            steps: Vec::new(),
            records_sequence: 0,
            fixture_gates,
            promoted: false,
        });
    }
    let world = DevScratchWorld::launch(&module, seed).await?;
    let initial_view = world.view().await?;
    let mut steps = Vec::with_capacity(inputs.len());
    for input in inputs {
        world.input(input.as_bytes()).await?;
        steps.push(DevStep {
            input: input.clone(),
            view: world.view().await?,
        });
    }
    let report = DevReport {
        world_id: DEV_WORLD_ID,
        dir: dir.to_path_buf(),
        source_files: sources.files.iter().map(|(name, _)| name.clone()).collect(),
        seed,
        module_bytes: module.len(),
        compiler_diagnostics: diagnostics,
        provenance: serde_json::to_value(&provenance)?,
        initial_view,
        steps,
        records_sequence: world.records_sequence(),
        fixture_gates,
        promoted: true,
    };
    // The scratch world ends with the session: dropping `world` here
    // discards its state, which is the Phase 1 lifecycle guarantee.
    drop(world);
    Ok(report)
}

/// Default debounce window for the watch loop (Phase 2): filesystem
/// events inside the window coalesce into one candidate build.
pub const DEFAULT_WATCH_DEBOUNCE_MS: u64 = 150;
/// Hard cap on watch polls per session (invariant 7: bounded work —
/// a watch session always terminates on its own).
pub const MAX_WATCH_CYCLES: u32 = 10_000;
/// Hard cap on the watch debounce window, in milliseconds.
pub const MAX_WATCH_DEBOUNCE_MS: u64 = 10_000;

impl DevSources {
    /// The source snapshot identity of this input set (invariant 2:
    /// one source snapshot per candidate). A deterministic content
    /// identifier over the length-prefixed `(name, content)` pairs in
    /// compile order, using the same canonical content hash as module
    /// identity ([`crate::world_session::module_hash`]). Compiler,
    /// fixture, and migration diagnostics for one candidate all name
    /// this snapshot; an edit anywhere in the set changes it, and the
    /// same set in a different order is a different snapshot.
    #[must_use]
    pub fn snapshot_id(&self) -> String {
        let mut bytes = Vec::new();
        for (name, content) in &self.files {
            bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&(content.len() as u64).to_le_bytes());
            bytes.extend_from_slice(content.as_bytes());
        }
        crate::world_session::module_hash(&bytes)
    }
}

/// Rescan `dir` (bounded, via [`discover_sources`]) and return the
/// fresh sources only when their snapshot differs from
/// `known_snapshot_id` — the watch loop's change test. `Ok(None)`
/// means the declared inputs are unchanged.
///
/// # Errors
///
/// Discovery fails exactly as [`discover_sources`] does; the caller
/// records the diagnostic and keeps the last-good snapshot.
pub fn rescan_changed(dir: &Path, known_snapshot_id: &str) -> Result<Option<DevSources>> {
    let sources = discover_sources(dir)?;
    if sources.snapshot_id() == known_snapshot_id {
        Ok(None)
    } else {
        Ok(Some(sources))
    }
}

/// The verdict of the post-compile freshness rescan in
/// [`run_dev_watch`]: what the sources are at *now*, after the
/// candidate was compiled. `Failed` carries the discovery diagnostic
/// — freshness could not be established, so the candidate must not
/// be promoted (issue #8781: a failed rescan was previously replaced
/// with the candidate's own snapshot id, which made the stale check
/// in [`DevSession::promote`] pass by construction and let the
/// candidate replace the last-good program while the sources were
/// undiscoverable).
#[derive(Clone, Debug, PartialEq, Eq)]
enum FreshnessVerdict {
    /// Discovery succeeded; the current snapshot id.
    Current(String),
    /// Discovery failed; the diagnostic.
    Failed(String),
}

/// Rescan `dir` for the post-compile freshness verdict. This never
/// substitutes any other snapshot id for a failed discovery: the
/// caller records a failed cycle and retains the active world.
fn freshness_verdict(dir: &Path) -> FreshnessVerdict {
    match discover_sources(dir) {
        Ok(fresh) => FreshnessVerdict::Current(fresh.snapshot_id()),
        Err(err) => FreshnessVerdict::Failed(format!("{err:#}")),
    }
}

/// The watch debouncer (Phase 2): a pure state machine over a
/// logical millisecond clock, so its behaviour is deterministic in
/// tests. Events noted inside the window coalesce; the pending
/// change becomes due one full window after the *last* event, so a
/// burst of saves yields one candidate build, not one per save.
#[derive(Clone, Debug)]
pub struct Debouncer {
    window_ms: u64,
    first_event_ms: Option<u64>,
    last_event_ms: Option<u64>,
    pending_events: usize,
    total_coalesced: usize,
}

impl Debouncer {
    /// A debouncer with the given window.
    #[must_use]
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            first_event_ms: None,
            last_event_ms: None,
            pending_events: 0,
            total_coalesced: 0,
        }
    }

    /// Record one change event at `now_ms`. An event arriving while
    /// another is already pending is coalesced into it.
    pub fn note_event(&mut self, now_ms: u64) {
        if self.pending_events > 0 {
            self.total_coalesced += 1;
        } else {
            self.first_event_ms = Some(now_ms);
        }
        self.pending_events += 1;
        self.last_event_ms = Some(now_ms);
    }

    /// True while a noted change has not been taken yet.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending_events > 0
    }

    /// True when a change is pending and a full window has passed
    /// since the last event.
    #[must_use]
    pub fn due(&self, now_ms: u64) -> bool {
        match self.last_event_ms {
            Some(last) => self.pending_events > 0 && now_ms.saturating_sub(last) >= self.window_ms,
            None => false,
        }
    }

    /// True when [`Debouncer::due`] holds, or when the pending
    /// change has been waiting at least `max_wait_ms` since the
    /// FIRST event of its burst. The trailing deadline alone is
    /// unbounded under continuous changes: every new event pushes
    /// it out, so a source that keeps changing faster than the
    /// window would never become due (Codex P2 on PR #8647). The
    /// max wait bounds the watch loop independently of source
    /// activity; the caller takes the newest snapshot when it
    /// fires, so no change is lost, only coalesced less.
    #[must_use]
    pub fn due_with_max_wait(&self, now_ms: u64, max_wait_ms: u64) -> bool {
        if self.due(now_ms) {
            return true;
        }
        match self.first_event_ms {
            Some(first) => self.pending_events > 0 && now_ms.saturating_sub(first) >= max_wait_ms,
            None => false,
        }
    }

    /// Take the pending change, returning how many events it
    /// coalesced (at least 1 when one was pending).
    pub fn take(&mut self) -> usize {
        let taken = self.pending_events;
        self.pending_events = 0;
        self.first_event_ms = None;
        self.last_event_ms = None;
        taken
    }

    /// Total events coalesced into an earlier pending change, over
    /// this debouncer's life.
    #[must_use]
    pub fn total_coalesced(&self) -> usize {
        self.total_coalesced
    }
}

/// The last-good diagnostics of a dev session (Phase 2): the active
/// candidate's identity plus the most recent promotion failure, if
/// any. A failed candidate never moves these values except
/// `last_failure` — that is what "last-good preservation" means
/// observably (invariant 3).
#[derive(Clone, Debug, Serialize)]
pub struct LastGood {
    /// Source snapshot identity of the active candidate.
    pub snapshot_id: String,
    /// Module hash of the active candidate.
    pub module_hash: String,
    /// Program version of the active candidate (bumps on each
    /// committed F12.9 upgrade).
    pub version: u32,
    /// Records sequence the active world has reached.
    pub records_sequence: u64,
    /// The most recent promotion failure's diagnostic, or `None`
    /// when the last promotion committed (or none was attempted).
    pub last_failure: Option<String>,
}

/// The outcome of one candidate promotion attempt (Phase 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionOutcome {
    /// The candidate passed the fixture gates and the F12.9 upgrade
    /// commit, replayed the session's input history to the migrated
    /// records, and is now the active program.
    Promoted,
    /// The candidate is the already-active snapshot; nothing to do.
    Unchanged,
    /// The sources changed again while the candidate was being
    /// built or validated, so the candidate was discarded without
    /// being promoted (invariant 2). The newest snapshot is built
    /// next instead.
    StaleDiscarded,
    /// A fixture gate blocked the candidate (Phase 3); the
    /// last-good program keeps running.
    FixtureBlocked,
    /// The F12.9 upgrade failed on its probe; the upgrade rolled
    /// back and the last-good program keeps running untouched.
    MigrationFailed,
    /// The candidate launched but replaying the session's input
    /// history did not reproduce the migrated records, so it was
    /// discarded; the last-good program keeps running.
    RecordsDiverged,
    /// The candidate module failed to launch in the sandbox; the
    /// last-good program keeps running.
    LaunchFailed,
    /// The candidate launched but its program became unhealthy while
    /// initializing or replaying; the last-good program keeps running.
    CandidateUnhealthy,
}

impl PromotionOutcome {
    /// The outcome's report name (the serde snake_case form).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Promoted => "promoted",
            Self::Unchanged => "unchanged",
            Self::StaleDiscarded => "stale_discarded",
            Self::FixtureBlocked => "fixture_blocked",
            Self::MigrationFailed => "migration_failed",
            Self::RecordsDiverged => "records_diverged",
            Self::LaunchFailed => "launch_failed",
            Self::CandidateUnhealthy => "candidate_unhealthy",
        }
    }
}

/// The outcome of one promotion attempt against a [`DevSession`].
#[derive(Clone, Debug, Serialize)]
pub struct PromotionReport {
    /// What happened to the candidate.
    pub outcome: PromotionOutcome,
    /// Source snapshot the candidate was built from.
    pub candidate_snapshot_id: String,
    /// Module hash of the candidate module.
    pub candidate_module_hash: String,
    /// Source snapshot of the active program after the attempt.
    pub active_snapshot_id: String,
    /// Program version of the active program after the attempt.
    pub active_version: u32,
    /// Fixture gates run against the candidate (empty when the
    /// candidate was discarded before validation).
    pub fixture_gates: Vec<FixtureGate>,
    /// The failure diagnostic, for non-committed outcomes.
    pub detail: Option<String>,
}

/// Run the F12.9 upgrade gate for a promotion: encode the active
/// records as the running program's state snapshot and upgrade the
/// program one version through [`crate::world::upgrade_program`].
///
/// The scratch world's guest exposes no `migrate` export through
/// the local actor seam, so the state is carried byte-for-byte
/// (`UpgradeOutcome::CarriedUnchanged` semantics); what the gate
/// contributes is F12.9's probe/commit contract — the migrated
/// state must round-trip through the current snapshot format
/// before anything commits, and any failure leaves the passed
/// program bit-for-bit unchanged (invariant 4: no alternate direct
/// state replacement).
///
/// # Errors
///
/// The records do not encode, or the F12.9 upgrade fails on its
/// probe (for example the state exceeds the snapshot bound); the
/// caller's active program is untouched either way.
fn migrate_active_state(
    records: &Records,
    from_version: u32,
) -> Result<crate::world::RunningProgram> {
    let state =
        serde_json::to_vec(records).context("encode the active records as upgrade state")?;
    let snapshot = crate::world::ProgramSnapshot::new(state)
        .encode()
        .context("encode the active program snapshot")?;
    let mut program = crate::world::RunningProgram {
        version: from_version,
        snapshot,
    };
    crate::world::upgrade_program(&mut program, from_version + 1, None)
        .context("F12.9 upgrade of the active program")?;
    Ok(program)
}

/// A live dev session (Phase 2): one active scratch world, the
/// input history applied to it, and the promotion machinery that
/// hot-swaps it to a new candidate without losing its state.
///
/// Promotion is transactional, in the design's order: stale check
/// (invariant 2) → fixture gates (Phase 3) → F12.9 probe/migrate/
/// commit over the active records (invariant 4) → candidate launch
/// and replay of the input history, which must reproduce the
/// migrated records exactly. Any failure discards the candidate and
/// records the diagnostic in the last-good diagnostics; the active
/// world is only replaced after every step has passed.
pub struct DevSession {
    world: DevScratchWorld,
    seed: u64,
    snapshot_id: String,
    module_hash: String,
    version: u32,
    inputs: Vec<Vec<u8>>,
    last_failure: Option<String>,
}

impl DevSession {
    /// Launch a session whose active program is `module`, built
    /// from the source snapshot `snapshot_id`. The program starts
    /// at version 1.
    ///
    /// # Errors
    ///
    /// The module fails to launch as a scratch world.
    pub async fn launch(module: &[u8], seed: u64, snapshot_id: String) -> Result<Self> {
        let world = DevScratchWorld::launch(module, seed).await?;
        Ok(Self {
            world,
            seed,
            snapshot_id,
            module_hash: crate::world_session::module_hash(module),
            version: 1,
            inputs: Vec::new(),
            last_failure: None,
        })
    }

    /// The active world's current view.
    ///
    /// # Errors
    ///
    /// The actor refuses or the guest call fails.
    pub async fn view(&self) -> Result<Option<String>> {
        self.world.view().await
    }

    /// Send one input to the active world and record it in the
    /// session history a future candidate must replay.
    ///
    /// # Errors
    ///
    /// The actor refuses or the guest call fails; a failed input is
    /// not recorded.
    pub async fn input(&mut self, input: &[u8]) -> Result<()> {
        self.world.input(input).await?;
        self.inputs.push(input.to_vec());
        Ok(())
    }

    /// The active world's current records.
    #[must_use]
    pub fn records(&self) -> Records {
        self.world.records()
    }

    /// The active program's version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The active candidate's source snapshot identity.
    #[must_use]
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// The session's last-good diagnostics.
    #[must_use]
    pub fn last_good(&self) -> LastGood {
        LastGood {
            snapshot_id: self.snapshot_id.clone(),
            module_hash: self.module_hash.clone(),
            version: self.version,
            records_sequence: self.world.records_sequence(),
            last_failure: self.last_failure.clone(),
        }
    }

    /// Attempt to hot-swap the active program to the candidate
    /// `module`, which was built from `candidate_snapshot_id` while
    /// the freshest rescan reports `current_snapshot_id`. See the
    /// type's docs for the step order; the returned report names the
    /// outcome and, on failure, the diagnostic — the active world is
    /// observably untouched in every failure case.
    pub async fn promote(
        &mut self,
        module: &[u8],
        candidate_snapshot_id: &str,
        current_snapshot_id: &str,
        fixtures: &[(String, Fixture)],
    ) -> PromotionReport {
        let candidate_module_hash = crate::world_session::module_hash(module);
        let report = |outcome: PromotionOutcome,
                      fixture_gates: Vec<FixtureGate>,
                      detail: Option<String>,
                      this: &Self| PromotionReport {
            outcome,
            candidate_snapshot_id: candidate_snapshot_id.to_owned(),
            candidate_module_hash: candidate_module_hash.clone(),
            active_snapshot_id: this.snapshot_id.clone(),
            active_version: this.version,
            fixture_gates,
            detail,
        };
        // Invariant 2: a candidate built from a superseded snapshot
        // is never promoted, however good it is.
        if candidate_snapshot_id != current_snapshot_id {
            let detail = format!(
                "candidate snapshot {candidate_snapshot_id} is stale; the sources are at {current_snapshot_id}"
            );
            self.last_failure = Some(detail.clone());
            return report(
                PromotionOutcome::StaleDiscarded,
                Vec::new(),
                Some(detail),
                self,
            );
        }
        if candidate_snapshot_id == self.snapshot_id {
            return report(PromotionOutcome::Unchanged, Vec::new(), None, self);
        }
        // Phase 3 promotion gates, unchanged: a mismatching fixture
        // blocks the candidate before anything migrates.
        let mut fixture_gates = Vec::with_capacity(fixtures.len());
        for (label, fixture) in fixtures {
            match fixture_gate(module, self.seed, label, fixture) {
                Ok(gate) => fixture_gates.push(gate),
                Err(err) => {
                    let detail = format!("fixture gate error: {err:#}");
                    self.last_failure = Some(detail.clone());
                    return report(
                        PromotionOutcome::FixtureBlocked,
                        fixture_gates,
                        Some(detail),
                        self,
                    );
                }
            }
        }
        if fixture_gates.iter().any(|gate| gate.blocks_promotion) {
            let blocking: Vec<&str> = fixture_gates
                .iter()
                .filter(|gate| gate.blocks_promotion)
                .map(|gate| gate.fixture.as_str())
                .collect();
            let detail = format!(
                "fixture gate blocked the candidate; mismatching fixture(s): {}",
                blocking.join(", ")
            );
            self.last_failure = Some(detail.clone());
            return report(
                PromotionOutcome::FixtureBlocked,
                fixture_gates,
                Some(detail),
                self,
            );
        }
        // Invariant 4: the state transition goes through F12.9's
        // probe/migrate/commit, or it does not happen at all.
        let program = match migrate_active_state(&self.world.records(), self.version) {
            Ok(program) => program,
            Err(err) => {
                let detail = format!("{err:#}");
                self.last_failure = Some(detail.clone());
                return report(
                    PromotionOutcome::MigrationFailed,
                    fixture_gates,
                    Some(detail),
                    self,
                );
            }
        };
        let migrated_records: Records =
            match crate::world::ProgramSnapshot::decode(&program.snapshot).and_then(|snapshot| {
                serde_json::from_slice(&snapshot.state).context("decode the migrated records")
            }) {
                Ok(records) => records,
                Err(err) => {
                    let detail = format!("migrated state does not decode: {err:#}");
                    self.last_failure = Some(detail.clone());
                    return report(
                        PromotionOutcome::MigrationFailed,
                        fixture_gates,
                        Some(detail),
                        self,
                    );
                }
            };
        // Launch the candidate and replay the session history: the
        // migrated state is only real if the new program reaches
        // exactly those records from the recorded inputs.
        let candidate = match DevScratchWorld::launch(module, self.seed).await {
            Ok(world) => world,
            Err(err) => {
                let detail = format!("candidate launch failed: {err:#}");
                self.last_failure = Some(detail.clone());
                return report(
                    PromotionOutcome::LaunchFailed,
                    fixture_gates,
                    Some(detail),
                    self,
                );
            }
        };
        let mut replay_error: Option<String> = None;
        for input in &self.inputs {
            if let Err(err) = candidate.input(input).await {
                replay_error = Some(format!("candidate replay of an input failed: {err:#}"));
                break;
            }
        }
        if let Some(detail) = replay_error {
            self.last_failure = Some(detail.clone());
            drop(candidate);
            return report(
                PromotionOutcome::LaunchFailed,
                fixture_gates,
                Some(detail),
                self,
            );
        }
        match candidate.program_healthy().await {
            Ok(true) => {}
            Ok(false) => {
                let detail =
                    "candidate program is unhealthy after replay; candidate discarded".to_owned();
                self.last_failure = Some(detail.clone());
                drop(candidate);
                return report(
                    PromotionOutcome::CandidateUnhealthy,
                    fixture_gates,
                    Some(detail),
                    self,
                );
            }
            Err(err) => {
                let detail = format!("candidate health check failed: {err:#}");
                self.last_failure = Some(detail.clone());
                drop(candidate);
                return report(
                    PromotionOutcome::CandidateUnhealthy,
                    fixture_gates,
                    Some(detail),
                    self,
                );
            }
        }
        if candidate.records() != migrated_records {
            let detail =
                "candidate replay does not reproduce the migrated records; candidate discarded"
                    .to_owned();
            self.last_failure = Some(detail.clone());
            drop(candidate);
            return report(
                PromotionOutcome::RecordsDiverged,
                fixture_gates,
                Some(detail),
                self,
            );
        }
        // Commit: the single point where the active program changes.
        self.world = candidate;
        self.version = program.version;
        self.snapshot_id = candidate_snapshot_id.to_owned();
        self.module_hash = candidate_module_hash.clone();
        self.last_failure = None;
        report(PromotionOutcome::Promoted, fixture_gates, None, self)
    }
}

/// One recorded change cycle of a watch session (Phase 2).
#[derive(Clone, Debug, Serialize)]
pub struct WatchCycle {
    /// Poll index at which the change was detected.
    pub cycle: u32,
    /// Source snapshot the cycle built (or tried to build).
    pub snapshot_id: String,
    /// Change events the debouncer coalesced into this cycle.
    pub coalesced_events: usize,
    /// The cycle's outcome: a [`PromotionOutcome`] name, or
    /// `rescan_failed` / `compile_failed` /
    /// `freshness_rescan_failed` for cycles that never reached
    /// promotion.
    pub outcome: String,
    /// The diagnostic, for failed cycles.
    pub detail: Option<String>,
    /// The active world's rendered view after this cycle, when the
    /// cycle promoted a new program (issue #8782); `None` for cycles
    /// that did not promote, and when the guest renders no view.
    pub view: Option<String>,
}

/// The outcome of a bounded watch session (Phase 2).
#[derive(Clone, Debug, Serialize)]
pub struct WatchReport {
    /// The scratch world's id (always [`DEV_WORLD_ID`]).
    pub world_id: &'static str,
    /// The app directory that was watched.
    pub dir: PathBuf,
    /// Seed the guest was initialized with.
    pub seed: u64,
    /// Source snapshot the session started from.
    pub initial_snapshot_id: String,
    /// True when the initial candidate was launched. False only
    /// when a fixture gate blocked it: nothing was launched and no
    /// watch cycle ran.
    pub initial_promoted: bool,
    /// The view before any initial input was sent (`null` when a
    /// fixture gate blocked the initial candidate: nothing was
    /// launched).
    pub initial_view: Option<String>,
    /// Initial inputs applied, in order, each with its resulting
    /// view -- the watch counterpart of [`DevReport`]'s steps, so a
    /// `--watch --input` user can observe the private scratch world
    /// after their inputs (issue #8782).
    pub steps: Vec<DevStep>,
    /// Fixture gates run against the initial candidate.
    pub fixture_gates: Vec<FixtureGate>,
    /// Polls performed (bounded by the caller's cycle budget).
    pub polls: u32,
    /// Distinct source snapshots detected across the polls.
    pub changes: u32,
    /// One entry per detected change, in order.
    pub cycles: Vec<WatchCycle>,
    /// Source snapshot of the active program at the end.
    pub active_snapshot_id: String,
    /// Program version of the active program at the end.
    pub active_version: u32,
    /// Last-good diagnostics at the end of the session.
    pub last_good: LastGood,
}

/// Compile one candidate from already-discovered `sources`,
/// returning the module, diagnostics, and provenance — the setup
/// [`run_dev_once`] performs, factored for the watch driver.
///
/// # Errors
///
/// Platform resolution, compiler setup, or compilation fails.
async fn compile_sources(
    sources: &DevSources,
    platform: Option<&Path>,
    roc: Option<&str>,
    seed: u64,
) -> Result<(Vec<u8>, String, serde_json::Value)> {
    let platform_dir = resolve_platform_dir(&sources.dir, platform, sources.main_source())?;
    let roc_bin = match roc {
        Some(roc) => roc.to_owned(),
        None => crate::world_compile::locate_roc()?,
    };
    let compiler = Compiler::new(platform_dir.clone(), roc_bin)?;
    let granted = platform_module_names(&platform_dir)?;
    let (module, diagnostics, provenance) =
        crate::world_compile::compile(&sources.spec(seed), &compiler, &granted, COMPILE_TIMEOUT)
            .await?;
    let provenance = serde_json::to_value(&provenance)?;
    Ok((module, diagnostics, provenance))
}

/// Decide the watch loop's known snapshot after a candidate has
/// been compiled: it stays the candidate, never the freshest rescan.
///
/// A snapshot becomes known only by being compiled as a candidate
/// itself. If the sources moved while the candidate compiled
/// (candidate B, current C), marking C known would strand it: the
/// promotion discards B as stale, and every later poll compares C
/// against itself, so C is never compiled and the old program keeps
/// running until another edit (issue #8779). Keeping the candidate
/// known means the next poll sees C as a change and compiles it; if
/// that compile fails, C is by then the candidate of its own cycle
/// and is marked known at that attempt — invariant 7 (a failing
/// snapshot is never rebuilt in a loop) still holds, per candidate.
#[must_use]
pub fn known_snapshot_after_candidate(
    candidate_snapshot_id: &str,
    current_snapshot_id: &str,
) -> String {
    let _ = current_snapshot_id;
    candidate_snapshot_id.to_owned()
}

/// Run a bounded watch session (Phase 2): launch the initial
/// candidate (behind the Phase 3 fixture gates), apply `inputs`,
/// then poll the declared inputs for changes. Each detected change
/// is debounced and coalesced, rescanned within the compiler's
/// bounds, compiled once, and promoted through [`DevSession`]'s
/// transactional hot-swap; a snapshot that fails any stage is
/// recorded in the cycle log and never retried (invariant 7: an
/// error does not create an infinite rebuild loop) — the next poll
/// watches for the *next* change from the last known snapshot.
///
/// The session is bounded by `max_cycles` polls of `debounce_ms`
/// (plus one debounce window per change); it is a long-running
/// session shape with a deterministic end, not an unbounded daemon.
///
/// # Errors
///
/// The budgets are out of bounds, or the initial discovery,
/// compilation, fixture gate, or launch fails. Per-change failures
/// after launch are cycle outcomes, not errors.
#[allow(clippy::too_many_arguments)]
pub async fn run_dev_watch(
    dir: &Path,
    platform: Option<&Path>,
    roc: Option<&str>,
    seed: u64,
    inputs: &[String],
    fixtures: &[(String, Fixture)],
    max_cycles: u32,
    debounce_ms: u64,
) -> Result<WatchReport> {
    ensure!(
        (1..=MAX_WATCH_CYCLES).contains(&max_cycles),
        "the watch cycle budget {max_cycles} is outside 1..={MAX_WATCH_CYCLES}"
    );
    ensure!(
        debounce_ms <= MAX_WATCH_DEBOUNCE_MS,
        "the watch debounce {debounce_ms}ms exceeds the bound {MAX_WATCH_DEBOUNCE_MS}ms"
    );
    let sources = discover_sources(dir)?;
    let initial_snapshot_id = sources.snapshot_id();
    let (module, _, _) = compile_sources(&sources, platform, roc, seed).await?;
    let mut fixture_gates = Vec::with_capacity(fixtures.len());
    for (label, fixture) in fixtures {
        fixture_gates.push(fixture_gate(&module, seed, label, fixture)?);
    }
    if fixture_gates.iter().any(|gate| gate.blocks_promotion) {
        return Ok(WatchReport {
            world_id: DEV_WORLD_ID,
            dir: dir.to_path_buf(),
            seed,
            initial_snapshot_id: initial_snapshot_id.clone(),
            initial_promoted: false,
            initial_view: None,
            steps: Vec::new(),
            fixture_gates,
            polls: 0,
            changes: 0,
            cycles: Vec::new(),
            active_snapshot_id: initial_snapshot_id.clone(),
            active_version: 0,
            last_good: LastGood {
                snapshot_id: initial_snapshot_id,
                module_hash: crate::world_session::module_hash(&module),
                version: 0,
                records_sequence: 0,
                last_failure: Some("fixture gate blocked the initial candidate".to_owned()),
            },
        });
    }
    let mut session = DevSession::launch(&module, seed, initial_snapshot_id.clone()).await?;
    // Rendering failures before the first poll are captured, not
    // fatal (Codex P2 on PR #8792): the program stays healthy after
    // a failed view, and exiting here would stop the user repairing
    // the rendering code through the watch -- an initial input can
    // even be what makes the view render again. The diagnostic
    // surfaces in the report through last_good.last_failure, the
    // same channel as the other recoverable session failures;
    // a later successful promotion clears it.
    let initial_view = match session.view().await {
        Ok(view) => view,
        Err(err) => {
            session.last_failure = Some(format!("initial view failed: {err:#}"));
            None
        }
    };
    let mut steps = Vec::with_capacity(inputs.len());
    for input in inputs {
        session.input(input.as_bytes()).await?;
        let view = match session.view().await {
            Ok(view) => view,
            Err(err) => {
                session.last_failure = Some(format!("view failed after input: {err:#}"));
                None
            }
        };
        steps.push(DevStep {
            input: input.clone(),
            view,
        });
    }
    let started = std::time::Instant::now();
    let now_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut known_snapshot_id = initial_snapshot_id.clone();
    let mut polls: u32 = 0;
    let mut changes: u32 = 0;
    let mut cycles: Vec<WatchCycle> = Vec::new();
    for cycle in 0..max_cycles {
        tokio::time::sleep(std::time::Duration::from_millis(debounce_ms.max(1))).await;
        polls += 1;
        let changed = match rescan_changed(dir, &known_snapshot_id) {
            Ok(changed) => changed,
            Err(err) => {
                cycles.push(WatchCycle {
                    cycle,
                    snapshot_id: known_snapshot_id.clone(),
                    coalesced_events: 0,
                    outcome: "rescan_failed".to_owned(),
                    detail: Some(format!("{err:#}")),
                    view: None,
                });
                continue;
            }
        };
        let Some(mut sources) = changed else {
            continue;
        };
        changes += 1;
        // Debounce + coalesce: keep rescanning inside the window;
        // the candidate is built from the newest snapshot only.
        // The wait is capped at one window from the first event of
        // the burst (due_with_max_wait): under continuous changes
        // the trailing deadline alone would never fire, and this
        // loop would run indefinitely inside one cycle, defeating
        // the max_cycles bound (invariant 7; Codex P2 on PR #8647).
        let mut debouncer = Debouncer::new(debounce_ms);
        debouncer.note_event(now_ms());
        while !debouncer.due_with_max_wait(now_ms(), debounce_ms) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if let Ok(Some(newer)) = rescan_changed(dir, &sources.snapshot_id()) {
                sources = newer;
                debouncer.note_event(now_ms());
            }
        }
        let coalesced_events = debouncer.take().saturating_sub(1);
        let candidate_snapshot_id = sources.snapshot_id();
        // The newest known snapshot moves first, whatever happens
        // next: a failing snapshot is reported once, never rebuilt
        // in a loop (invariant 7).
        known_snapshot_id = candidate_snapshot_id.clone();
        let module = match compile_sources(&sources, platform, roc, seed).await {
            Ok((module, _, _)) => module,
            Err(err) => {
                let detail = format!("{err:#}");
                session.last_failure = Some(detail.clone());
                cycles.push(WatchCycle {
                    cycle,
                    snapshot_id: candidate_snapshot_id,
                    coalesced_events,
                    outcome: "compile_failed".to_owned(),
                    detail: Some(detail),
                    view: None,
                });
                continue;
            }
        };
        // Fresh rescan for the stale check: what the sources are at
        // *now*, after the compile, is what the candidate must match.
        // A failed rescan establishes nothing about freshness, so it
        // must never be treated as "unchanged": record a failed
        // cycle and retain the active world instead of promoting
        // (issue #8781).
        let current_snapshot_id = match freshness_verdict(dir) {
            FreshnessVerdict::Current(current_snapshot_id) => current_snapshot_id,
            FreshnessVerdict::Failed(detail) => {
                session.last_failure = Some(detail.clone());
                cycles.push(WatchCycle {
                    cycle,
                    snapshot_id: candidate_snapshot_id,
                    coalesced_events,
                    outcome: "freshness_rescan_failed".to_owned(),
                    detail: Some(detail),
                    view: None,
                });
                continue;
            }
        };
        // The freshest snapshot becomes known only by being
        // compiled as a candidate itself (#8779): if the sources
        // moved past the candidate during its compile, promotion
        // discards the candidate as stale — leaving known at the
        // candidate here is what makes the next poll compile the
        // freshest snapshot instead of comparing it against itself.
        known_snapshot_id =
            known_snapshot_after_candidate(&candidate_snapshot_id, &current_snapshot_id);
        let promotion = session
            .promote(
                &module,
                &candidate_snapshot_id,
                &current_snapshot_id,
                fixtures,
            )
            .await;
        // A rendering failure after a successful promotion is a
        // cycle diagnostic, not a session failure (Codex P2 on
        // PR #8792): the program stays healthy after a failed view,
        // so record the error on the cycle and keep polling -- the
        // `?` form terminated the watch and discarded the whole
        // report. The promotion itself stands; only the view is
        // missing, and the next change can fix the rendering code.
        let mut detail = promotion.detail;
        let view = if promotion.outcome == PromotionOutcome::Promoted {
            match session.view().await {
                Ok(view) => view,
                Err(err) => {
                    detail = Some(format!("view failed after promotion: {err:#}"));
                    None
                }
            }
        } else {
            None
        };
        cycles.push(WatchCycle {
            cycle,
            snapshot_id: candidate_snapshot_id,
            coalesced_events,
            outcome: promotion.outcome.as_str().to_owned(),
            detail,
            view,
        });
    }
    Ok(WatchReport {
        world_id: DEV_WORLD_ID,
        dir: dir.to_path_buf(),
        seed,
        initial_snapshot_id,
        initial_promoted: true,
        initial_view,
        steps,
        fixture_gates,
        polls,
        changes,
        cycles,
        active_snapshot_id: session.snapshot_id().to_owned(),
        active_version: session.version(),
        last_good: session.last_good(),
    })
}

/// Largest fuzz declaration file the parser accepts, in bytes.
pub const MAX_FUZZ_DECL_BYTES: usize = 64 * 1024;
/// Hard cap on generated cases per campaign (invariant 7: bounded work).
pub const MAX_FUZZ_CASES: u32 = 10_000;
/// Hard cap on inputs generated for one case.
pub const MAX_FUZZ_SEQUENCE: usize = 64;
/// Hard cap on the declared input alphabet's size.
const MAX_FUZZ_INPUTS: usize = 32;
/// Hard cap on one declared input's length, in bytes.
const MAX_FUZZ_INPUT_BYTES: usize = 128;
/// Hard cap on declared invariants per file.
const MAX_FUZZ_INVARIANTS: usize = 32;
/// Hard cap on keys in one `keys_present` invariant.
const MAX_FUZZ_KEYS: usize = 64;

/// One declared invariant in a fuzz declaration file (Phase 4).
///
/// The v1 set is deliberately narrow and machine-checkable, per the
/// design: guest calls do not trap, records remain valid JSON,
/// declared record keys remain present, and declared numeric
/// bounds/order relations hold. A numeric invariant naming a key the
/// records do not hold, or holding a non-numeric value, is a
/// violation: the declaration asserts the key's presence and shape.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FuzzInvariant {
    /// Applying any generated input must not error (trap/refuse),
    /// including in the post-input `after_change` processing the
    /// input triggers (`plaza_wants`).
    NoTrap,
    /// The records must serialize as JSON after every input.
    RecordsValidJson,
    /// Every named key is present in the records after every input.
    KeysPresent {
        /// Record keys that must remain present.
        keys: Vec<String>,
    },
    /// The named record key is numeric and stays within the bounds
    /// (`min`/`max` are inclusive; either may be omitted).
    NumericBounds {
        /// The record key checked.
        key: String,
        /// Inclusive lower bound, if any.
        min: Option<serde_json::Number>,
        /// Inclusive upper bound, if any. Bounds keep their declared
        /// JSON form so an integer bound above 2^53 is not rounded
        /// before it is compared (issue #8752).
        max: Option<serde_json::Number>,
    },
    /// Both named keys are numeric and `first <= second` holds.
    NumericOrder {
        /// The key that must be the smaller (or equal) value.
        first: String,
        /// The key that must be the larger (or equal) value.
        second: String,
    },
}

impl FuzzInvariant {
    /// The declaration-format name of this invariant's kind.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NoTrap => "no_trap",
            Self::RecordsValidJson => "records_valid_json",
            Self::KeysPresent { .. } => "keys_present",
            Self::NumericBounds { .. } => "numeric_bounds",
            Self::NumericOrder { .. } => "numeric_order",
        }
    }

    /// Check this invariant against `records` (the state after one
    /// applied input). Returns a human-readable violation detail.
    fn check(&self, records: &Records) -> std::result::Result<(), String> {
        match self {
            Self::NoTrap => Ok(()),
            Self::RecordsValidJson => serde_json::to_string(records)
                .map(|_| ())
                .map_err(|err| format!("records do not serialize as JSON: {err}")),
            Self::KeysPresent { keys } => {
                for key in keys {
                    if !records.contains_key(key) {
                        return Err(format!("declared key {key:?} is absent from the records"));
                    }
                }
                Ok(())
            }
            Self::NumericBounds { key, min, max } => {
                let value = numeric(records, key)?;
                if let Some(min) = min
                    && value.cmp_exact(&NumericValue::from_number(min)) == Ordering::Less
                {
                    return Err(format!("{key} = {value} is below the declared min {min}"));
                }
                if let Some(max) = max
                    && value.cmp_exact(&NumericValue::from_number(max)) == Ordering::Greater
                {
                    return Err(format!("{key} = {value} exceeds the declared max {max}"));
                }
                Ok(())
            }
            Self::NumericOrder { first, second } => {
                let a = numeric(records, first)?;
                let b = numeric(records, second)?;
                if a.cmp_exact(&b) == Ordering::Greater {
                    return Err(format!(
                        "declared order violated: {first} = {a} > {second} = {b}"
                    ));
                }
                Ok(())
            }
        }
    }
}

/// A numeric value for invariant checks, kept exact.
///
/// Record values and declared bounds are `serde_json` numbers: an
/// integer above 2^53 is stored exactly there, but `as_f64()` rounds
/// it, so comparing through `f64` can read two different integers as
/// equal and let a violated invariant pass (issue #8752). Integers
/// are therefore compared as `i128` (every `serde_json` integer --
/// `i64` or `u64` -- fits), and a float is compared against an
/// integer exactly: an integral float converts to `i128`, and a
/// fractional float has magnitude below 2^52 (floats at or above
/// 2^52 are all integral), so the integer's relation to the float's
/// ceiling decides the order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericValue {
    Int(i128),
    Float(f64),
}

impl NumericValue {
    fn from_number(number: &serde_json::Number) -> Self {
        if let Some(value) = number.as_i64() {
            Self::Int(i128::from(value))
        } else if let Some(value) = number.as_u64() {
            Self::Int(i128::from(value))
        } else {
            // A serde_json number that is not an integer is a float.
            Self::Float(number.as_f64().unwrap_or(f64::NAN))
        }
    }

    fn cmp_exact(&self, other: &Self) -> Ordering {
        match (*self, *other) {
            (Self::Int(a), Self::Int(b)) => a.cmp(&b),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            (Self::Int(a), Self::Float(b)) => cmp_int_float(a, b),
            (Self::Float(a), Self::Int(b)) => cmp_int_float(b, a).reverse(),
        }
    }
}

impl std::fmt::Display for NumericValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

/// Exact ordering of an integer against a float (see [`NumericValue`]).
fn cmp_int_float(int: i128, float: f64) -> Ordering {
    if float.is_nan() {
        // Unreachable for serde_json numbers; never let an
        // unorderable value pass silently as equal-or-less.
        return Ordering::Greater;
    }
    if float.trunc() == float {
        // Integral float: exact in i128 when in range; far outside
        // the i128 range the float's sign alone decides the order.
        if (-1.7e38..1.7e38).contains(&float) {
            int.cmp(&(float as i128))
        } else if float > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    } else if int < float.ceil() as i128 {
        // Fractional float (|float| < 2^52, so the ceiling is exact):
        // int < float iff int < ceil(float); equality is impossible.
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// The numeric value of a declared record key, for invariant checks.
fn numeric(records: &Records, key: &str) -> std::result::Result<NumericValue, String> {
    match records.get(key) {
        None => Err(format!("declared key {key:?} is absent from the records")),
        Some(value) => value
            .as_number()
            .map(NumericValue::from_number)
            .ok_or_else(|| format!("declared key {key:?} is not numeric: {value}")),
    }
}

/// A parsed fuzz declaration file (Phase 4): the bounded generated
/// input alphabet plus the declared invariants a candidate must
/// hold under generated inputs.
///
/// Format (version 1), JSON:
///
/// ```json
/// {
///   "version": 1,
///   "inputs": ["inc", "dec"],
///   "invariants": [
///     {"kind": "no_trap"},
///     {"kind": "keys_present", "keys": ["count"]},
///     {"kind": "numeric_bounds", "key": "count", "min": 0, "max": 10}
///   ]
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct FuzzDecl {
    /// Format version; only `1` is understood.
    pub version: u32,
    /// The input alphabet: generated cases are sequences over these
    /// input texts.
    pub inputs: Vec<String>,
    /// The declared invariants, checked after every applied input.
    #[serde(default)]
    pub invariants: Vec<FuzzInvariant>,
}

/// Parse and bound-check a fuzz declaration file's text.
///
/// # Errors
///
/// The text is not valid JSON in the version-1 shape, the version is
/// not 1, or the declaration exceeds its bounds (alphabet size, input
/// length, invariant count, key count).
pub fn parse_fuzz_decl(text: &str) -> Result<FuzzDecl> {
    ensure!(
        text.len() <= MAX_FUZZ_DECL_BYTES,
        "fuzz declaration exceeds {MAX_FUZZ_DECL_BYTES} bytes"
    );
    let decl: FuzzDecl =
        serde_json::from_str(text).context("parse the fuzz declaration as version-1 JSON")?;
    ensure!(
        decl.version == 1,
        "unsupported fuzz declaration version {}",
        decl.version
    );
    ensure!(
        !decl.inputs.is_empty(),
        "the fuzz declaration names no inputs"
    );
    ensure!(
        decl.inputs.len() <= MAX_FUZZ_INPUTS,
        "the fuzz declaration names {} inputs; the bound is {MAX_FUZZ_INPUTS}",
        decl.inputs.len()
    );
    for input in &decl.inputs {
        ensure!(
            input.len() <= MAX_FUZZ_INPUT_BYTES,
            "a declared fuzz input exceeds {MAX_FUZZ_INPUT_BYTES} bytes"
        );
    }
    ensure!(
        decl.invariants.len() <= MAX_FUZZ_INVARIANTS,
        "the fuzz declaration names {} invariants; the bound is {MAX_FUZZ_INVARIANTS}",
        decl.invariants.len()
    );
    for invariant in &decl.invariants {
        if let FuzzInvariant::KeysPresent { keys } = invariant {
            ensure!(
                keys.len() <= MAX_FUZZ_KEYS,
                "a keys_present invariant names {} keys; the bound is {MAX_FUZZ_KEYS}",
                keys.len()
            );
        }
        if let FuzzInvariant::NumericBounds { min, max, .. } = invariant
            && let (Some(min), Some(max)) = (min, max)
        {
            ensure!(
                NumericValue::from_number(min).cmp_exact(&NumericValue::from_number(max))
                    != Ordering::Greater,
                "a numeric_bounds invariant has min {min} above max {max}"
            );
        }
    }
    Ok(decl)
}

/// A SplitMix64 generator: the campaign's only randomness source.
/// Pure `u64` arithmetic, so a `(seed, case)` pair produces the same
/// sequence on every platform and run (invariant 6). Host entropy is
/// never consulted.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// The generated input sequence for one case of a campaign.
///
/// Deterministic in `(decl, fuzz_seed, case)`: the generator is
/// seeded per case, so a case's sequence does not depend on the
/// campaign budget — the same seed and case index reproduce the same
/// sequence in a longer or shorter run (invariant 6).
#[must_use]
pub fn fuzz_case_inputs(decl: &FuzzDecl, fuzz_seed: u64, case: u32) -> Vec<String> {
    let mut rng = SplitMix64(fuzz_seed ^ u64::from(case).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let len = 1 + rng.below(MAX_FUZZ_SEQUENCE);
    let mut inputs = Vec::with_capacity(len);
    for _ in 0..len {
        inputs.push(decl.inputs[rng.below(decl.inputs.len())].clone());
    }
    inputs
}

/// The first invariant failure of a fuzz campaign: the reproducible
/// counterexample. Re-running the campaign with the same declaration,
/// fuzz seed, and case budget reports this same failure (invariant
/// 6); applying `inputs` (up to and including the failing input) to a
/// fresh candidate reproduces it directly.
#[derive(Clone, Debug, Serialize)]
pub struct FuzzFailure {
    /// Index of the failing case within the campaign.
    pub case: u32,
    /// Index of the failing input within the case's sequence.
    pub step: usize,
    /// The case's input sequence up to and including the failing input.
    pub inputs: Vec<String>,
    /// The violated invariant's kind (`no_trap`, `keys_present`, ...).
    pub invariant: String,
    /// What violated the invariant.
    pub detail: String,
    /// The candidate's records at the failure (after the failing
    /// input; the pre-input records when the input itself trapped).
    pub records: Records,
}

/// The outcome of one deterministic fuzz campaign (Phase 4).
#[derive(Clone, Debug, Serialize)]
pub struct FuzzReport {
    /// The fuzz seed the campaign was generated from.
    pub fuzz_seed: u64,
    /// Cases the campaign set out to run (the bounded budget).
    pub cases_budgeted: u32,
    /// Cases actually run (the budgeted count, or the failing case
    /// index + 1 when the campaign stopped at a failure).
    pub cases_run: u32,
    /// Inputs applied across all run cases, including a failing one.
    pub inputs_applied: u64,
    /// Number of invariants declared.
    pub invariants: usize,
    /// The first failure, when one was found.
    pub failure: Option<FuzzFailure>,
}

impl FuzzReport {
    /// The one-line reproducible summary of the first failure, in
    /// the shape the CLI prints and bails with; `None` when the
    /// campaign passed.
    #[must_use]
    pub fn failure_summary(&self) -> Option<String> {
        self.failure.as_ref().map(|failure| {
            format!(
                "fuzz seed {} case {} step {}: invariant {} violated: {} (inputs: {:?})",
                self.fuzz_seed,
                failure.case,
                failure.step,
                failure.invariant,
                failure.detail,
                failure.inputs
            )
        })
    }
}

/// Run a deterministic fuzz campaign against the compiled candidate
/// `module`: generate `budget` cases from `decl` under `fuzz_seed`,
/// apply each case's inputs to a fresh scratch world (initialized
/// with `seed`), and check the declared invariants after every
/// input. The campaign stops at the first failure and reports it as
/// the counterexample; it never loops past `budget` cases
/// (invariants 6 and 7).
///
/// # Errors
///
/// The budget is zero or above [`MAX_FUZZ_CASES`], or launching a
/// scratch world fails. A guest input that errors is reported as a
/// `no_trap` failure, not an error: trapping is an outcome the
/// campaign exists to find. A guest that traps in the post-input
/// processing an input triggers (`plaza_wants`, run by the actor
/// after the input reply) is likewise a `no_trap` failure at that
/// input's step: the campaign checks program health after every
/// input, and the health query is answered only after the post-input
/// pass completes.
pub async fn fuzz_candidate(
    module: &[u8],
    seed: u64,
    decl: &FuzzDecl,
    fuzz_seed: u64,
    budget: u32,
) -> Result<FuzzReport> {
    ensure!(budget >= 1, "the fuzz case budget must be at least 1");
    ensure!(
        budget <= MAX_FUZZ_CASES,
        "the fuzz case budget {budget} exceeds the bound {MAX_FUZZ_CASES}"
    );
    // Compile the candidate once for the whole campaign; each case
    // still runs against a fresh isolated guest instance (a new
    // store, memory, and world), so per-case isolation is unchanged.
    let runner = Sandbox::compile(module, SandboxLimits::default())?;
    let mut inputs_applied: u64 = 0;
    for case in 0..budget {
        let inputs = fuzz_case_inputs(decl, fuzz_seed, case);
        let world = DevScratchWorld::launch_compiled(&runner, seed).await?;
        let mut failure: Option<FuzzFailure> = None;
        for (step, input) in inputs.iter().enumerate() {
            let before = world.records();
            if let Err(err) = world.input(input.as_bytes()).await {
                failure = Some(FuzzFailure {
                    case,
                    step,
                    inputs: inputs[..=step].to_vec(),
                    invariant: "no_trap".to_owned(),
                    detail: format!("guest call for input {input:?} failed: {err}"),
                    records: before,
                });
                inputs_applied += 1;
                break;
            }
            inputs_applied += 1;
            let records = world.records();
            // The input reply is sent before the actor's post-input
            // `after_change` pass runs `plaza_wants`; a trap there
            // only latches `program_healthy = false` in the actor and
            // would otherwise surface, if at all, as the NEXT input's
            // failure — never when the trapping input is the last one
            // (#8751). The health query is answered in actor order,
            // after that pass, so this is where a post-input trap
            // becomes visible: report it as this step's `no_trap`
            // failure, with the records the input left behind.
            if !world.program_healthy().await? {
                failure = Some(FuzzFailure {
                    case,
                    step,
                    inputs: inputs[..=step].to_vec(),
                    invariant: "no_trap".to_owned(),
                    detail: format!(
                        "guest trapped in post-input processing (plaza_wants) after input {input:?}"
                    ),
                    records,
                });
                break;
            }
            for invariant in &decl.invariants {
                if let Err(detail) = invariant.check(&records) {
                    failure = Some(FuzzFailure {
                        case,
                        step,
                        inputs: inputs[..=step].to_vec(),
                        invariant: invariant.kind().to_owned(),
                        detail,
                        records,
                    });
                    break;
                }
            }
            if failure.is_some() {
                break;
            }
        }
        drop(world);
        if let Some(failure) = failure {
            return Ok(FuzzReport {
                fuzz_seed,
                cases_budgeted: budget,
                cases_run: case + 1,
                inputs_applied,
                invariants: decl.invariants.len(),
                failure: Some(failure),
            });
        }
    }
    Ok(FuzzReport {
        fuzz_seed,
        cases_budgeted: budget,
        cases_run: budget,
        inputs_applied,
        invariants: decl.invariants.len(),
        failure: None,
    })
}

/// Compile the app in `dir` exactly as [`run_dev_once`] does and run
/// a deterministic fuzz campaign against the compiled candidate.
///
/// # Errors
///
/// Discovery, platform/compiler setup, compilation, or the campaign
/// itself fails (see [`parse_fuzz_decl`] and [`fuzz_candidate`]). A
/// found invariant failure is a report, not an error; the caller
/// decides how to present it.
pub async fn run_dev_fuzz(
    dir: &Path,
    platform: Option<&Path>,
    roc: Option<&str>,
    seed: u64,
    decl: &FuzzDecl,
    fuzz_seed: u64,
    budget: u32,
) -> Result<FuzzReport> {
    let sources = discover_sources(dir)?;
    let platform_dir = resolve_platform_dir(dir, platform, sources.main_source())?;
    let roc_bin = match roc {
        Some(roc) => roc.to_owned(),
        None => crate::world_compile::locate_roc()?,
    };
    let compiler = Compiler::new(platform_dir.clone(), roc_bin)?;
    let granted = platform_module_names(&platform_dir)?;
    let (module, _, _) =
        crate::world_compile::compile(&sources.spec(seed), &compiler, &granted, COMPILE_TIMEOUT)
            .await?;
    fuzz_candidate(&module, seed, decl, fuzz_seed, budget).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_platform_path_reads_the_header() {
        let main = "app [main!] { pf: platform \"../roc-world/platform.roc\", other: \"x\" }\n\nmain! = |_| \"hi\"\n";
        assert_eq!(
            declared_platform_path(main).as_deref(),
            Some("../roc-world/platform.roc")
        );
        // A `platform` string in the body is not a declaration.
        let body_only =
            "app [main!] { pf: platform2 \"nope\" }\n\nx = \"platform \\\"later\\\"\"\n";
        assert_eq!(declared_platform_path(body_only), None);
        assert_eq!(declared_platform_path("app [main!] {}\n"), None);
    }

    #[test]
    fn discover_sources_orders_main_first_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.roc"),
            "app [main!] { pf: platform \"../roc-world/platform.roc\" }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("zed.roc"), "Zed := [].\n").unwrap();
        std::fs::write(dir.path().join("alpha.roc"), "Alpha := [].\n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a source\n").unwrap();
        let sources = discover_sources(dir.path()).unwrap();
        let names: Vec<&str> = sources
            .files
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["main.roc", "alpha.roc", "zed.roc"]);
        assert_eq!(
            resolve_platform_dir(dir.path(), None, sources.main_source()).unwrap(),
            dir.path().join("../roc-world")
        );
    }

    #[test]
    fn discover_sources_requires_main_roc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other.roc"), "Other := [].\n").unwrap();
        let error = discover_sources(dir.path()).unwrap_err().to_string();
        assert!(error.contains("no main.roc"), "{error}");
    }

    #[tokio::test]
    async fn scratch_world_launches_applies_inputs_and_starts_fresh_each_time() {
        // Oracle: the checked-in Roc counter example. Its view renders
        // `count=N`; `inc` inputs increment it. A scratch world must
        // start at the compiled program's initial state, apply inputs
        // through the actor, and leave nothing behind for the next
        // launch (disposability / no remote side effects).
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let world = DevScratchWorld::launch(wasm, 7).await.unwrap();
        assert!(
            world
                .view()
                .await
                .unwrap()
                .is_some_and(|v| v.contains("count=0"))
        );
        world.input(b"inc").await.unwrap();
        world.input(b"inc").await.unwrap();
        assert!(
            world
                .view()
                .await
                .unwrap()
                .is_some_and(|v| v.contains("count=2"))
        );
        drop(world);

        let fresh = DevScratchWorld::launch(wasm, 7).await.unwrap();
        assert!(
            fresh
                .view()
                .await
                .unwrap()
                .is_some_and(|v| v.contains("count=0"))
        );
    }

    /// A complete fixture over the counter example: two `inc` inputs,
    /// with the records a live scratch world reaches after them.
    /// Oracle: the live world's own records, captured before the
    /// fixture exists — the gate must reproduce exactly those.
    async fn counter_fixture(
        wasm: &[u8],
        module_hash: Option<String>,
    ) -> (Fixture, crate::world::Records) {
        let world = DevScratchWorld::launch(wasm, 7).await.unwrap();
        world.input(b"inc").await.unwrap();
        world.input(b"inc").await.unwrap();
        let records = world.records();
        drop(world);
        let fixture = Fixture {
            header: crate::world_fixture::FixtureHeader {
                world_id: DEV_WORLD_ID.to_owned(),
                module_hash,
                current_sequence: 2,
                complete: true,
                records: Some(records.clone()),
            },
            events: vec![
                crate::world_fixture::FixtureEvent::Input {
                    sequence: 1,
                    participant_id: 1,
                    bytes: b"inc".to_vec(),
                },
                crate::world_fixture::FixtureEvent::Input {
                    sequence: 2,
                    participant_id: 1,
                    bytes: b"inc".to_vec(),
                },
            ],
        };
        (fixture, records)
    }

    #[tokio::test]
    async fn fixture_gate_promotes_a_candidate_that_reproduces_the_fixture() {
        // Oracle: the fixture's recorded expectation is the live
        // counter world's records after the same inputs; a candidate
        // that replays to those records matches and does not block.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let (fixture, _) =
            counter_fixture(wasm, Some(crate::world_session::module_hash(wasm))).await;
        let gate = fixture_gate(wasm, 7, "counter.jsonl", &fixture).unwrap();
        assert!(!gate.cross_module);
        assert!(gate.complete);
        assert_eq!(gate.verdict, "match");
        assert_eq!(gate.verdict_reason, None);
        assert!(!gate.blocks_promotion);
    }

    #[tokio::test]
    async fn fixture_gate_blocks_a_candidate_whose_records_diverge() {
        // Oracle: the same fixture with a tampered expectation — the
        // replayed records cannot equal it, so the gate mismatches
        // and blocks promotion (the design's promotion-gate case).
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let (mut fixture, _) =
            counter_fixture(wasm, Some(crate::world_session::module_hash(wasm))).await;
        fixture
            .header
            .records
            .as_mut()
            .unwrap()
            .insert("gate-oracle".to_owned(), serde_json::json!("tampered"));
        let gate = fixture_gate(wasm, 7, "counter.jsonl", &fixture).unwrap();
        assert_eq!(gate.verdict, "mismatch");
        assert!(gate.blocks_promotion);
    }

    #[tokio::test]
    async fn fixture_gate_replays_a_fixture_recorded_under_another_module() {
        // Oracle: invariant 5's dev regression mode — a fixture
        // recorded under a different module hash still replays in
        // dev (ordinary replay/verify refuse via check_module), the
        // gate reports the cross-module fact, and the comparison
        // itself still decides promotion.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let other = crate::world_session::module_hash(b"a different module entirely");
        let (fixture, _) = counter_fixture(wasm, Some(other)).await;
        let gate = fixture_gate(wasm, 7, "counter.jsonl", &fixture).unwrap();
        assert!(gate.cross_module);
        assert_eq!(gate.verdict, "match");
        assert!(!gate.blocks_promotion);
    }

    #[tokio::test]
    async fn fixture_gate_reports_a_fixture_with_no_expectation() {
        // Oracle: a fixture that records no expected records has
        // nothing to compare — the gate must say not_compared with
        // the reason instead of passing silently, and must not block.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let (mut fixture, _) =
            counter_fixture(wasm, Some(crate::world_session::module_hash(wasm))).await;
        fixture.header.records = None;
        let gate = fixture_gate(wasm, 7, "counter.jsonl", &fixture).unwrap();
        assert_eq!(gate.verdict, "not_compared");
        assert_eq!(
            gate.verdict_reason.as_deref(),
            Some("the fixture has no records")
        );
        assert!(!gate.blocks_promotion);
    }
}

#[cfg(test)]
mod fuzz_tests {
    use super::*;

    fn counter_decl(invariants: Vec<FuzzInvariant>) -> FuzzDecl {
        FuzzDecl {
            version: 1,
            inputs: vec!["inc".to_owned(), "dec".to_owned(), "bogus".to_owned()],
            invariants,
        }
    }

    #[test]
    fn parse_fuzz_decl_accepts_the_version_1_shape() {
        let decl = parse_fuzz_decl(
            r#"{
                "version": 1,
                "inputs": ["inc", "dec"],
                "invariants": [
                    {"kind": "no_trap"},
                    {"kind": "records_valid_json"},
                    {"kind": "keys_present", "keys": ["count"]},
                    {"kind": "numeric_bounds", "key": "count", "min": 0, "max": 10},
                    {"kind": "numeric_order", "first": "a", "second": "b"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(decl.inputs, ["inc", "dec"]);
        assert_eq!(decl.invariants.len(), 5);
        assert_eq!(decl.invariants[2].kind(), "keys_present");
        assert_eq!(decl.invariants[3].kind(), "numeric_bounds");
    }

    #[test]
    fn parse_fuzz_decl_refuses_out_of_bound_declarations() {
        // Oracle: each refusal names the bound the declaration broke;
        // an out-of-bound declaration must never reach the generator
        // (invariant 7: bounded work).
        let cases: Vec<String> = vec![
            r#"{"version": 2, "inputs": ["inc"]}"#.to_owned(),
            r#"{"version": 1, "inputs": []}"#.to_owned(),
            format!(
                r#"{{"version": 1, "inputs": [{}]}}"#,
                (0..MAX_FUZZ_INPUTS + 1)
                    .map(|i| format!("\"in{i}\""))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            format!(
                r#"{{"version": 1, "inputs": ["{}"]}}"#,
                "x".repeat(MAX_FUZZ_INPUT_BYTES + 1)
            ),
            r#"{"version": 1, "inputs": ["inc"], "invariants": [{"kind": "numeric_bounds", "key": "count", "min": 5, "max": 1}]}"#.to_owned(),
            r#"{"version": 1, "inputs": ["inc"], "invariants": [{"kind": "not_a_kind"}]}"#.to_owned(),
        ];
        for text in &cases {
            assert!(parse_fuzz_decl(text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn numeric_invariants_compare_integers_above_2_to_the_53_exactly() {
        // Oracle: issue #8752 -- 9007199254740993 (2^53 + 1) and
        // 9007199254740992 (2^53) are equal as f64 but different
        // integers; the invariants must see the difference.
        let records: Records = [
            ("a".to_owned(), serde_json::json!(9_007_199_254_740_993_i64)),
            ("b".to_owned(), serde_json::json!(9_007_199_254_740_992_i64)),
        ]
        .into_iter()
        .collect();
        let order = FuzzInvariant::NumericOrder {
            first: "a".to_owned(),
            second: "b".to_owned(),
        };
        assert!(
            order.check(&records).is_err(),
            "numeric_order must report a > b above 2^53"
        );
        let order_ok = FuzzInvariant::NumericOrder {
            first: "b".to_owned(),
            second: "a".to_owned(),
        };
        assert!(order_ok.check(&records).is_ok());
        let bounds = FuzzInvariant::NumericBounds {
            key: "a".to_owned(),
            min: None,
            max: Some(serde_json::Number::from(9_007_199_254_740_992_i64)),
        };
        assert!(
            bounds.check(&records).is_err(),
            "numeric_bounds max 2^53 must reject the record value 2^53 + 1"
        );
        let bounds_ok = FuzzInvariant::NumericBounds {
            key: "a".to_owned(),
            min: Some(serde_json::Number::from(9_007_199_254_740_992_i64)),
            max: Some(serde_json::Number::from(9_007_199_254_740_993_i64)),
        };
        assert!(bounds_ok.check(&records).is_ok());
        // A bound declared in JSON text keeps its exact integer form
        // through parsing (it is not rounded to f64 at parse time).
        let decl = parse_fuzz_decl(
            r#"{"version": 1, "inputs": ["inc"], "invariants": [{"kind": "numeric_bounds", "key": "a", "max": 9007199254740992}]}"#,
        )
        .unwrap();
        assert!(decl.invariants[0].check(&records).is_err());
        // Ordinary float values and bounds still compare as before.
        let float_records: Records = [
            ("x".to_owned(), serde_json::json!(1.5)),
            ("y".to_owned(), serde_json::json!(2)),
        ]
        .into_iter()
        .collect();
        let float_order = FuzzInvariant::NumericOrder {
            first: "x".to_owned(),
            second: "y".to_owned(),
        };
        assert!(float_order.check(&float_records).is_ok());
        let float_order_bad = FuzzInvariant::NumericOrder {
            first: "y".to_owned(),
            second: "x".to_owned(),
        };
        assert!(float_order_bad.check(&float_records).is_err());
    }

    /// An exactly known generated number: the `serde_json::Number`
    /// fed to the comparator, paired with an independent exact
    /// oracle for its value. The oracle is built from the
    /// generation parameters -- never from the number's float bits
    /// and never via `NumericValue` -- so it is a second,
    /// independent statement of the ordering. (The Codex P1 finding
    /// on PR #8755 asked for property coverage of exactly this
    /// shape: mixed signed/unsigned integers and floats, negative
    /// values, unsigned extremes, integral-float boundaries, and
    /// subnormals, checked against an independent oracle.)
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum ExactOracle {
        /// The value is exactly `n / 8`.
        Eighths(i128),
        /// The value is `sign * 2^-1074` (the subnormal
        /// `f64::MIN_POSITIVE` and its negation): strictly between
        /// 0 and 1/8 in magnitude, so it orders against every
        /// `Eighths` value by sign and zeroness alone.
        Tiny(i128),
    }

    impl ExactOracle {
        fn cmp_oracle(&self, other: &Self) -> Ordering {
            match (*self, *other) {
                (Self::Eighths(a), Self::Eighths(b)) => a.cmp(&b),
                (Self::Tiny(a), Self::Tiny(b)) => a.cmp(&b),
                (Self::Tiny(sign), Self::Eighths(b)) => {
                    if sign > 0 {
                        if b <= 0 {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        }
                    } else if b >= 0 {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (Self::Eighths(a), Self::Tiny(sign)) => {
                    Self::Tiny(sign).cmp_oracle(&Self::Eighths(a)).reverse()
                }
            }
        }
    }

    fn exact_number_strategy()
    -> impl proptest::strategy::Strategy<Value = (serde_json::Number, ExactOracle)> {
        use proptest::prelude::*;
        prop_oneof![
            // Any signed integer.
            any::<i64>().prop_map(|v| {
                (
                    serde_json::Number::from(v),
                    ExactOracle::Eighths(i128::from(v) * 8),
                )
            }),
            // Any unsigned integer (covers u64::MAX, far above 2^53).
            any::<u64>().prop_map(|v| {
                (
                    serde_json::Number::from(v),
                    ExactOracle::Eighths(i128::from(v) * 8),
                )
            }),
            // Boundary integers around 2^52 / 2^53 and the extremes.
            proptest::sample::select(vec![
                0i128,
                1,
                -1,
                (1 << 52) - 1,
                1 << 52,
                (1 << 53) - 1,
                1 << 53,
                (1 << 53) + 1,
                -((1 << 53) - 1),
                -(1 << 53),
                -((1 << 53) + 1),
                i128::from(i64::MIN),
                i128::from(i64::MAX),
                i128::from(u64::MAX),
            ])
            .prop_map(|v| {
                let number = if (i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(&v) {
                    serde_json::Number::from(v as i64)
                } else {
                    serde_json::Number::from(v as u64)
                };
                (number, ExactOracle::Eighths(v * 8))
            }),
            // Multiples of 1/8 across the full i32 numerator range:
            // exact as f64 (|n| < 2^31 keeps n / 8 far from any
            // rounding), covering fractional and integral floats.
            any::<i32>().prop_map(|n| {
                (
                    serde_json::Number::from_f64(f64::from(n) / 8.0).unwrap(),
                    ExactOracle::Eighths(i128::from(n)),
                )
            }),
            // Integral floats at and above the 2^52 boundary: the
            // base carries at most 40 significant bits and is
            // shifted left, so the f64 is exact and its integer
            // value is base << shift.
            (-1_000_000_000_000i64..=1_000_000_000_000i64, 0u32..=22u32).prop_map(
                |(base, shift)| {
                    let float = (base as f64) * f64::powi(2.0, i32::try_from(shift).unwrap());
                    (
                        serde_json::Number::from_f64(float).unwrap(),
                        ExactOracle::Eighths((i128::from(base) << shift) * 8),
                    )
                }
            ),
            // Subnormals: the smallest positive f64 and its negation.
            proptest::sample::select(vec![1i128, -1]).prop_map(|sign| {
                let float = if sign > 0 {
                    f64::MIN_POSITIVE
                } else {
                    -f64::MIN_POSITIVE
                };
                (
                    serde_json::Number::from_f64(float).unwrap(),
                    ExactOracle::Tiny(sign),
                )
            }),
        ]
    }

    proptest::proptest! {
        #[test]
        fn numeric_cmp_exact_matches_the_independent_oracle(
            a in exact_number_strategy(),
            b in exact_number_strategy(),
        ) {
            // Oracle: `ExactOracle::cmp_oracle`, derived from the
            // generation parameters of the two generated numbers.
            let (a_number, a_oracle) = a;
            let (b_number, b_oracle) = b;
            let expected = a_oracle.cmp_oracle(&b_oracle);
            let a_value = NumericValue::from_number(&a_number);
            let b_value = NumericValue::from_number(&b_number);
            proptest::prop_assert_eq!(
                a_value.cmp_exact(&b_value),
                expected,
                "{} vs {}",
                a_number,
                b_number
            );
            // Ordering symmetry: reversing the operands reverses
            // the ordering, never changes its kind.
            proptest::prop_assert_eq!(b_value.cmp_exact(&a_value), expected.reverse());
            // The invariant surface must agree with the comparator:
            // numeric_order holds iff a <= b.
            let records: Records = [
                ("a".to_owned(), serde_json::Value::Number(a_number)),
                ("b".to_owned(), serde_json::Value::Number(b_number)),
            ]
            .into_iter()
            .collect();
            let order = FuzzInvariant::NumericOrder {
                first: "a".to_owned(),
                second: "b".to_owned(),
            };
            proptest::prop_assert_eq!(
                order.check(&records).is_ok(),
                expected != Ordering::Greater
            );
        }

        #[test]
        fn numeric_bounds_agree_between_declaration_validation_and_record_checks(
            min in exact_number_strategy(),
            max in exact_number_strategy(),
            value in exact_number_strategy(),
        ) {
            let (min_number, min_oracle) = min;
            let (max_number, max_oracle) = max;
            let (value_number, value_oracle) = value;
            let text = format!(
                r#"{{"version": 1, "inputs": ["inc"], "invariants": [{{"kind": "numeric_bounds", "key": "count", "min": {min_number}, "max": {max_number}}}]}}"#
            );
            // Declaration validation and record checks must use the
            // same exact ordering: the declaration parses iff
            // min <= max, and a parsed declaration then accepts a
            // record iff min <= value <= max (same oracle).
            let parsed = parse_fuzz_decl(&text);
            proptest::prop_assert_eq!(
                parsed.is_ok(),
                min_oracle.cmp_oracle(&max_oracle) != Ordering::Greater,
                "decl {}", text
            );
            if let Ok(decl) = parsed {
                let records: Records =
                    [("count".to_owned(), serde_json::Value::Number(value_number))]
                        .into_iter()
                        .collect();
                let within = min_oracle.cmp_oracle(&value_oracle) != Ordering::Greater
                    && value_oracle.cmp_oracle(&max_oracle) != Ordering::Greater;
                proptest::prop_assert_eq!(
                    decl.invariants[0].check(&records).is_ok(),
                    within,
                    "decl {}", text
                );
            }
        }
    }

    #[test]
    fn fuzz_case_inputs_are_deterministic_and_budget_independent() {
        // Oracle: invariant 6 — the same (decl, seed, case) yields
        // the same sequence regardless of anything else, and two
        // different cases (or seeds) do not collapse onto one
        // sequence.
        let decl = counter_decl(vec![]);
        let a = fuzz_case_inputs(&decl, 42, 3);
        let b = fuzz_case_inputs(&decl, 42, 3);
        assert_eq!(a, b);
        assert!(!a.is_empty() && a.len() <= MAX_FUZZ_SEQUENCE);
        assert!(a.iter().all(|input| decl.inputs.contains(input)));
        assert_ne!(a, fuzz_case_inputs(&decl, 42, 4));
        assert_ne!(a, fuzz_case_inputs(&decl, 43, 3));
    }

    #[tokio::test]
    async fn fuzz_campaign_passes_a_candidate_that_holds_its_invariants() {
        // Oracle: the counter example under a bound its reachable
        // counts cannot break within one case (a case applies at
        // most MAX_FUZZ_SEQUENCE inputs, so count <= 64 < 1000):
        // the campaign must run the whole budget and find nothing.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let decl = counter_decl(vec![
            FuzzInvariant::NoTrap,
            FuzzInvariant::RecordsValidJson,
            FuzzInvariant::KeysPresent {
                keys: vec!["count".to_owned()],
            },
            FuzzInvariant::NumericBounds {
                key: "count".to_owned(),
                min: Some(serde_json::Number::from(0)),
                max: Some(serde_json::Number::from(1000)),
            },
        ]);
        let report = fuzz_candidate(wasm, 7, &decl, 42, 8).await.unwrap();
        assert!(report.failure.is_none(), "{report:?}");
        assert_eq!(report.cases_run, 8);
        assert!(report.inputs_applied >= 8);
    }

    #[tokio::test]
    async fn fuzz_campaign_fails_when_guest_traps_in_post_input_wants() {
        // Oracle: issue #8751 — the input reply is sent before the
        // actor's `after_change` pass calls `plaza_wants`, so a guest
        // whose wants traps only after an input used to pass `no_trap`
        // (the trap surfaced, if at all, as the NEXT input's failure —
        // and never when the trapping input was a case's last one).
        // This guest accepts every input and traps in `plaza_wants`
        // from the first input on: the campaign must report a
        // `no_trap` failure at step 0 of case 0.
        let wasm = wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1 1)
              (global (mut i32) (i32.const 0))
              (data (i32.const 32) "{}")
              (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
              (func (export "plaza_free") (param i32 i32))
              (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
              (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
                (global.set 0 (i32.const 1))
                local.get 0)
              (func (export "plaza_view") (param i32) (result i32) (i32.const 32))
              (func (export "plaza_records") (param i32) (result i32) (i32.const 32))
              (func (export "plaza_wants") (param i32) (result i32)
                (if (global.get 0) (then unreachable))
                (i32.const 32))
              (func (export "plaza_out_len") (result i32) (i32.const 2)))
            "#,
        )
        .unwrap();
        let decl = FuzzDecl {
            version: 1,
            inputs: vec!["inc".to_owned()],
            invariants: vec![FuzzInvariant::NoTrap],
        };
        let report = fuzz_candidate(&wasm, 7, &decl, 42, 4).await.unwrap();
        let failure = report
            .failure
            .expect("a post-input trap must fail the campaign");
        assert_eq!(failure.invariant, "no_trap");
        assert_eq!(failure.case, 0);
        assert_eq!(failure.step, 0);
        assert!(failure.detail.contains("post-input"), "{}", failure.detail);
        assert_eq!(report.cases_run, 1);
    }

    #[tokio::test]
    async fn fuzz_campaign_catches_the_seeded_faulty_pair_at_a_fixed_seed() {
        // The seeded-bug regression test the design requires: the
        // counter example paired with a bound it does not enforce
        // (count <= 2, while `inc` increments without limit) is a
        // deliberately faulty example/invariant pair. At the fixed
        // seed the campaign must catch it, deterministically, and
        // the reported counterexample must replay to the violation:
        // applying the reported inputs to a fresh world reaches
        // count = 3, the first value above the bound.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let decl = counter_decl(vec![FuzzInvariant::NumericBounds {
            key: "count".to_owned(),
            min: Some(serde_json::Number::from(0)),
            max: Some(serde_json::Number::from(2)),
        }]);
        let report = fuzz_candidate(wasm, 7, &decl, 42, 200).await.unwrap();
        let failure = report
            .failure
            .clone()
            .expect("the seeded faulty pair must be caught within the budget");
        assert_eq!(failure.invariant, "numeric_bounds");
        assert_eq!(failure.records.get("count"), Some(&serde_json::json!(3)));
        // Determinism: the identical campaign reports the identical
        // first failure (invariant 6).
        let again = fuzz_candidate(wasm, 7, &decl, 42, 200).await.unwrap();
        let again = again.failure.expect("deterministic rerun must fail too");
        assert_eq!(again.case, failure.case);
        assert_eq!(again.step, failure.step);
        assert_eq!(again.inputs, failure.inputs);
        // Replay oracle: the reported sequence, applied fresh,
        // reaches exactly the reported violating records.
        let world = DevScratchWorld::launch(wasm, 7).await.unwrap();
        for input in &failure.inputs {
            world.input(input.as_bytes()).await.unwrap();
        }
        assert_eq!(world.records().get("count"), Some(&serde_json::json!(3)));
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;

    fn sources(files: Vec<(&str, &str)>) -> DevSources {
        DevSources {
            dir: PathBuf::from("/tmp/dev-watch-test"),
            files: files
                .into_iter()
                .map(|(name, content)| (name.to_owned(), content.to_owned()))
                .collect(),
        }
    }

    #[test]
    fn snapshot_id_is_deterministic_and_content_sensitive() {
        // Oracle: invariant 2 — the same input set in the same
        // order is the same snapshot on every call; any content
        // edit, and any reordering, is a different snapshot.
        let a = sources(vec![("main.roc", "app x\n"), ("b.roc", "B := [].\n")]);
        let same = sources(vec![("main.roc", "app x\n"), ("b.roc", "B := [].\n")]);
        let edited = sources(vec![("main.roc", "app y\n"), ("b.roc", "B := [].\n")]);
        let reordered = sources(vec![("b.roc", "B := [].\n"), ("main.roc", "app x\n")]);
        assert_eq!(a.snapshot_id(), same.snapshot_id());
        assert_ne!(a.snapshot_id(), edited.snapshot_id());
        assert_ne!(a.snapshot_id(), reordered.snapshot_id());
    }

    #[test]
    fn debouncer_coalesces_a_burst_into_one_due_change() {
        // Oracle: the design's debounce contract — events inside
        // the window coalesce, nothing is due before a full quiet
        // window after the last event, and taking resets the state.
        let mut debouncer = Debouncer::new(150);
        assert!(!debouncer.due(1_000));
        debouncer.note_event(1_000);
        debouncer.note_event(1_050);
        debouncer.note_event(1_100);
        assert!(debouncer.is_pending());
        assert!(!debouncer.due(1_200));
        assert!(!debouncer.due(1_249));
        assert!(debouncer.due(1_250));
        assert_eq!(debouncer.take(), 3);
        assert_eq!(debouncer.total_coalesced(), 2);
        assert!(!debouncer.is_pending());
        assert!(!debouncer.due(9_999));
        assert_eq!(debouncer.take(), 0);
    }

    #[test]
    fn debouncer_max_wait_bounds_a_continuous_burst() {
        // Oracle: invariant 7 (bounded work) and the Codex P2 on
        // PR #8647 -- events arriving faster than the window keep
        // resetting the trailing deadline, so due() alone never
        // fires; the max wait measured from the FIRST event of the
        // burst must still make the change due, so the watch
        // session terminates within its budget under continuous
        // source changes (issue #8780).
        let mut debouncer = Debouncer::new(150);
        debouncer.note_event(1_000);
        // A new event every 50ms, faster than the 150ms window:
        // the trailing deadline never arrives.
        for t in [1_050, 1_100] {
            debouncer.note_event(t);
            assert!(!debouncer.due(t), "trailing deadline must not fire at {t}");
            assert!(
                !debouncer.due_with_max_wait(t, 150),
                "max wait not yet reached at {t}"
            );
        }
        // One full window after the first event the burst is due
        // even though the last event is only 50ms old.
        assert!(!debouncer.due(1_150));
        assert!(debouncer.due_with_max_wait(1_150, 150));
        // The whole burst is one coalesced change.
        assert_eq!(debouncer.take(), 3);
        // A later burst gets a fresh max wait from its own first
        // event, not from the earlier burst.
        debouncer.note_event(5_000);
        assert!(!debouncer.due_with_max_wait(5_100, 150));
        assert!(debouncer.due_with_max_wait(5_150, 150));
        assert_eq!(debouncer.take(), 1);
    }

    #[test]
    fn rescan_changed_reports_only_a_real_change() {
        // Oracle: a rescan over an unchanged directory finds no new
        // snapshot; editing a declared source produces the fresh
        // set whose snapshot differs.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n").unwrap();
        let known = discover_sources(dir.path()).unwrap().snapshot_id();
        assert!(rescan_changed(dir.path(), &known).unwrap().is_none());
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n\nx = 1\n").unwrap();
        let changed = rescan_changed(dir.path(), &known)
            .unwrap()
            .expect("an edit must rescan as a change");
        assert_ne!(changed.snapshot_id(), known);
    }

    #[test]
    fn freshness_verdict_fails_when_sources_become_undiscoverable() {
        // Oracle: issue #8781's acceptance — when the post-compile
        // freshness rescan cannot discover the sources (here:
        // main.roc deleted during compilation), the verdict is
        // Failed carrying the diagnostic, never a snapshot id the
        // candidate could match; run_dev_watch records a
        // `freshness_rescan_failed` cycle on this verdict and does
        // not promote, so the active world is retained.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n").unwrap();
        let known = discover_sources(dir.path()).unwrap().snapshot_id();
        assert_eq!(
            freshness_verdict(dir.path()),
            FreshnessVerdict::Current(known)
        );
        std::fs::remove_file(dir.path().join("main.roc")).unwrap();
        match freshness_verdict(dir.path()) {
            FreshnessVerdict::Failed(detail) => {
                assert!(detail.contains("main.roc"), "{detail}");
            }
            FreshnessVerdict::Current(id) => {
                panic!("a failed discovery must not report a current snapshot, got {id}")
            }
        }
    }

    #[test]
    fn snapshot_saved_during_a_stale_compile_is_the_next_change() {
        // Oracle: issue #8779 — snapshot C is saved while candidate
        // B compiles, and B is then discarded as stale. The watch
        // loop's known snapshot after B's compile must not be C:
        // the next poll's rescan over a directory still at C must
        // report C as a change, so C is compiled with no further
        // edit. (Before the fix the loop set known to C, the rescan
        // compared C against itself, and the old program kept
        // running until another edit.)
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n").unwrap();
        let snapshot_a = discover_sources(dir.path()).unwrap().snapshot_id();
        // Edit to B — the candidate the loop compiles.
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n\nx = 1\n").unwrap();
        let candidate_b = discover_sources(dir.path()).unwrap().snapshot_id();
        assert_ne!(candidate_b, snapshot_a);
        // Edit to C while B compiles; the freshest rescan is C.
        std::fs::write(dir.path().join("main.roc"), "app [main!] {}\n\nx = 2\n").unwrap();
        let current_c = discover_sources(dir.path()).unwrap().snapshot_id();
        assert_ne!(current_c, candidate_b);
        let known = known_snapshot_after_candidate(&candidate_b, &current_c);
        assert_eq!(known, candidate_b);
        let next = rescan_changed(dir.path(), &known)
            .unwrap()
            .expect("snapshot C must rescan as the next change");
        assert_eq!(next.snapshot_id(), current_c);
        // Once C itself has been the candidate, it is known: a
        // further rescan finds nothing, so no snapshot is rebuilt
        // in a loop (invariant 7).
        let known_after_c = known_snapshot_after_candidate(&current_c, &current_c);
        assert!(
            rescan_changed(dir.path(), &known_after_c)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn migrate_active_state_carries_records_through_the_f12_9_gate() {
        // Oracle: F12.9's own invariant — the upgrade commits a
        // program whose snapshot decodes to exactly the migrated
        // state (here: the records, carried byte-for-byte), at the
        // next version. An over-bound state fails on the probe and
        // commits nothing.
        let mut records = Records::new();
        records.insert("count".to_owned(), serde_json::json!(2));
        let program = migrate_active_state(&records, 1).unwrap();
        assert_eq!(program.version, 2);
        let decoded = crate::world::ProgramSnapshot::decode(&program.snapshot).unwrap();
        let round_tripped: Records = serde_json::from_slice(&decoded.state).unwrap();
        assert_eq!(round_tripped, records);

        let mut huge = Records::new();
        huge.insert(
            "blob".to_owned(),
            serde_json::json!("x".repeat(crate::world::MAX_PROGRAM_STATE_BYTES + 1)),
        );
        assert!(migrate_active_state(&huge, 1).is_err());
    }

    #[tokio::test]
    async fn promote_hot_swaps_and_preserves_the_session_state() {
        // Oracle: the counter world's records after two `inc`
        // inputs are the migrated state; a same-behaviour candidate
        // built from the current snapshot must promote, bump the
        // F12.9 version, and keep count=2 live — a restart-from-zero
        // (count=0) would prove the swap discarded the state.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let mut session = DevSession::launch(wasm, 7, "snap-a".to_owned())
            .await
            .unwrap();
        session.input(b"inc").await.unwrap();
        session.input(b"inc").await.unwrap();
        assert_eq!(session.records().get("count"), Some(&serde_json::json!(2)));

        let report = session.promote(wasm, "snap-b", "snap-b", &[]).await;
        assert_eq!(report.outcome, PromotionOutcome::Promoted, "{report:?}");
        assert_eq!(report.active_version, 2);
        assert_eq!(session.version(), 2);
        assert_eq!(session.snapshot_id(), "snap-b");
        assert_eq!(session.records().get("count"), Some(&serde_json::json!(2)));
        assert!(
            session
                .view()
                .await
                .unwrap()
                .is_some_and(|view| view.contains("count=2"))
        );
        assert_eq!(session.last_good().last_failure, None);

        // The now-active snapshot promoted again is a no-op.
        let report = session.promote(wasm, "snap-b", "snap-b", &[]).await;
        assert_eq!(report.outcome, PromotionOutcome::Unchanged);
        assert_eq!(session.version(), 2);
    }

    #[test]
    fn watch_report_serializes_the_views_it_carries() {
        // Oracle: issue #8782 at the output boundary -- the CLI
        // prints the watch report as JSON, so a view the report
        // carries must appear in that JSON under `initial_view`,
        // `steps[].view`, and `cycles[].view`; a fixture-blocked
        // report carries `initial_view: null` and no steps.
        let report = WatchReport {
            world_id: DEV_WORLD_ID,
            dir: PathBuf::from("/tmp/dev-watch-test"),
            seed: 7,
            initial_snapshot_id: "snap-a".to_owned(),
            initial_promoted: true,
            initial_view: Some("count=0".to_owned()),
            steps: vec![DevStep {
                input: "inc".to_owned(),
                view: Some("count=1".to_owned()),
            }],
            fixture_gates: Vec::new(),
            polls: 3,
            changes: 1,
            cycles: vec![WatchCycle {
                cycle: 0,
                snapshot_id: "snap-b".to_owned(),
                coalesced_events: 0,
                outcome: "promoted".to_owned(),
                detail: None,
                view: Some("count=1".to_owned()),
            }],
            active_snapshot_id: "snap-b".to_owned(),
            active_version: 2,
            last_good: LastGood {
                snapshot_id: "snap-b".to_owned(),
                module_hash: "hash".to_owned(),
                version: 2,
                records_sequence: 1,
                last_failure: None,
            },
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["initial_view"], "count=0");
        assert_eq!(json["steps"][0]["input"], "inc");
        assert_eq!(json["steps"][0]["view"], "count=1");
        assert_eq!(json["cycles"][0]["view"], "count=1");
    }

    #[tokio::test]
    async fn watch_report_carries_views_after_inputs_and_promotion() {
        // Oracle: issue #8782 -- the checked-in Roc counter example
        // renders `count=N`; a `--watch --input inc` user must see
        // that rendering in the watch output, first after the
        // initial inputs and again after a promotion, because the
        // scratch world is private and never served. Skips where
        // the roc toolchain or the built platform host is absent,
        // like the world_compile tests.
        if crate::world_compile::locate_roc().is_err() {
            eprintln!("skipping: no roc toolchain located");
            return;
        }
        let platform_src = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/roc-world"
        ));
        if !platform_src.join("targets/wasm32/host.wasm").is_file() {
            eprintln!("skipping: roc-world platform host.wasm not built");
            return;
        }
        let counter_main = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/roc-counter/main.roc"
        ))
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("main.roc"), &counter_main).unwrap();
        std::os::unix::fs::symlink(&platform_src, root.path().join("roc-world")).unwrap();

        // Edit the source while the watch polls: a comment-only
        // change is a new snapshot that compiles to the same
        // behaviour, so the promotion must succeed and its cycle
        // must carry the preserved state's view (count=2).
        let editor_app = app.clone();
        let editor = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let mut text = std::fs::read_to_string(editor_app.join("main.roc")).unwrap();
            text.push_str("\n# watch views test edit\n");
            std::fs::write(editor_app.join("main.roc"), text).unwrap();
        });

        let inputs = vec!["inc".to_owned(), "inc".to_owned()];
        let report = run_dev_watch(&app, None, None, 7, &inputs, &[], 200, 20)
            .await
            .unwrap();
        editor.await.unwrap();

        assert!(report.initial_promoted, "{report:?}");
        assert!(
            report
                .initial_view
                .as_deref()
                .is_some_and(|view| view.contains("count=0")),
            "initial_view: {:?}",
            report.initial_view
        );
        assert_eq!(report.steps.len(), 2, "{report:?}");
        assert!(
            report.steps[1]
                .view
                .as_deref()
                .is_some_and(|view| view.contains("count=2")),
            "step view after inputs: {:?}",
            report.steps[1].view
        );
        let promoted: Vec<&WatchCycle> = report
            .cycles
            .iter()
            .filter(|cycle| cycle.outcome == "promoted")
            .collect();
        assert!(!promoted.is_empty(), "cycles: {:?}", report.cycles);
        assert!(
            promoted[0]
                .view
                .as_deref()
                .is_some_and(|view| view.contains("count=2")),
            "promoted cycle view: {:?}",
            promoted[0].view
        );
        // The serialized report is the CLI's output: the views must
        // survive into it under their field names.
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"initial_view\""), "{json}");
        assert!(json.contains("\"view\""), "{json}");
    }

    #[tokio::test]
    async fn promote_discards_a_stale_candidate_and_keeps_last_good() {
        // Oracle: invariant 2 + 3 — a candidate built from snap-b
        // while the sources already sit at snap-c is discarded
        // without touching the active world, and the discard is
        // recorded in the last-good diagnostics.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let mut session = DevSession::launch(wasm, 7, "snap-a".to_owned())
            .await
            .unwrap();
        session.input(b"inc").await.unwrap();
        let report = session.promote(wasm, "snap-b", "snap-c", &[]).await;
        assert_eq!(report.outcome, PromotionOutcome::StaleDiscarded);
        assert_eq!(session.snapshot_id(), "snap-a");
        assert_eq!(session.version(), 1);
        assert_eq!(session.records().get("count"), Some(&serde_json::json!(1)));
        assert!(
            session
                .last_good()
                .last_failure
                .is_some_and(|failure| failure.contains("stale"))
        );
    }

    #[tokio::test]
    async fn promote_keeps_last_good_when_a_fixture_gate_blocks() {
        // Oracle: the Phase 3 gate inside promotion — a fixture
        // whose recorded expectation the candidate cannot reproduce
        // blocks the swap; the active world keeps its records and
        // version.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let mut session = DevSession::launch(wasm, 7, "snap-a".to_owned())
            .await
            .unwrap();
        session.input(b"inc").await.unwrap();
        session.input(b"inc").await.unwrap();
        let mut expected = session.records();
        expected.insert("gate-oracle".to_owned(), serde_json::json!("tampered"));
        let fixture = Fixture {
            header: crate::world_fixture::FixtureHeader {
                world_id: DEV_WORLD_ID.to_owned(),
                module_hash: Some(crate::world_session::module_hash(wasm)),
                current_sequence: 2,
                complete: true,
                records: Some(expected),
            },
            events: vec![
                crate::world_fixture::FixtureEvent::Input {
                    sequence: 1,
                    participant_id: 1,
                    bytes: b"inc".to_vec(),
                },
                crate::world_fixture::FixtureEvent::Input {
                    sequence: 2,
                    participant_id: 1,
                    bytes: b"inc".to_vec(),
                },
            ],
        };
        let fixtures = vec![("counter.jsonl".to_owned(), fixture)];
        let report = session.promote(wasm, "snap-b", "snap-b", &fixtures).await;
        assert_eq!(
            report.outcome,
            PromotionOutcome::FixtureBlocked,
            "{report:?}"
        );
        assert_eq!(session.snapshot_id(), "snap-a");
        assert_eq!(session.version(), 1);
        assert_eq!(session.records().get("count"), Some(&serde_json::json!(2)));
    }

    #[tokio::test]
    async fn promote_keeps_last_good_when_the_candidate_cannot_launch() {
        // Oracle: invariant 3 for the launch stage — module bytes
        // the sandbox refuses never become the active program.
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm");
        let mut session = DevSession::launch(wasm, 7, "snap-a".to_owned())
            .await
            .unwrap();
        session.input(b"inc").await.unwrap();
        let report = session
            .promote(b"not a wasm module", "snap-b", "snap-b", &[])
            .await;
        assert_eq!(report.outcome, PromotionOutcome::LaunchFailed, "{report:?}");
        assert_eq!(session.snapshot_id(), "snap-a");
        assert_eq!(session.records().get("count"), Some(&serde_json::json!(1)));
    }

    #[tokio::test]
    async fn promote_discards_unhealthy_candidate_with_empty_session_records() {
        // Oracle: an unhealthy candidate must never replace the last-good
        // world, even when its failed records export was substituted with
        // `{}` and therefore equals the empty migrated session state.
        let module_with_records = |records: &str| {
            wat::parse_str(format!(
                r#"
                (module
                  (memory (export "memory") 1 1)
                  (data (i32.const 32) "{records}")
                  (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
                  (func (export "plaza_free") (param i32 i32))
                  (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
                  (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
                    (local.get 0))
                  (func (export "plaza_view") (param i32) (result i32) (i32.const 32))
                  (func (export "plaza_records") (param i32) (result i32) (i32.const 32))
                  (func (export "plaza_wants") (param i32) (result i32) (i32.const 32))
                  (func (export "plaza_out_len") (result i32) (i32.const 2)))
                "#
            ))
            .unwrap()
        };
        let initial = module_with_records("{}");
        let unhealthy_candidate = module_with_records("[]");
        let mut session = DevSession::launch(&initial, 7, "snap-a".to_owned())
            .await
            .unwrap();

        let report = session
            .promote(&unhealthy_candidate, "snap-b", "snap-b", &[])
            .await;

        assert_eq!(
            report.outcome,
            PromotionOutcome::CandidateUnhealthy,
            "{report:?}"
        );
        assert_eq!(session.snapshot_id(), "snap-a");
        assert_eq!(session.version(), 1);
        assert!(session.records().is_empty());
    }
}
