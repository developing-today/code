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
//! Later phases (watch/debounce + F12.9 hot-swap) build on
//! [`DevScratchWorld`] and [`discover_sources`]; they are
//! deliberately not stubbed here.

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
    /// Applying any generated input must not error (trap/refuse).
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
        min: Option<f64>,
        /// Inclusive upper bound, if any.
        max: Option<f64>,
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
                    && value < *min
                {
                    return Err(format!("{key} = {value} is below the declared min {min}"));
                }
                if let Some(max) = max
                    && value > *max
                {
                    return Err(format!("{key} = {value} exceeds the declared max {max}"));
                }
                Ok(())
            }
            Self::NumericOrder { first, second } => {
                let a = numeric(records, first)?;
                let b = numeric(records, second)?;
                if a > b {
                    return Err(format!(
                        "declared order violated: {first} = {a} > {second} = {b}"
                    ));
                }
                Ok(())
            }
        }
    }
}

/// The numeric value of a declared record key, for invariant checks.
fn numeric(records: &Records, key: &str) -> std::result::Result<f64, String> {
    match records.get(key) {
        None => Err(format!("declared key {key:?} is absent from the records")),
        Some(value) => value
            .as_f64()
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
                min <= max,
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
/// campaign exists to find.
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
    let mut inputs_applied: u64 = 0;
    for case in 0..budget {
        let inputs = fuzz_case_inputs(decl, fuzz_seed, case);
        let world = DevScratchWorld::launch(module, seed).await?;
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let decl = counter_decl(vec![
            FuzzInvariant::NoTrap,
            FuzzInvariant::RecordsValidJson,
            FuzzInvariant::KeysPresent {
                keys: vec!["count".to_owned()],
            },
            FuzzInvariant::NumericBounds {
                key: "count".to_owned(),
                min: Some(0.0),
                max: Some(1000.0),
            },
        ]);
        let report = fuzz_candidate(wasm, 7, &decl, 42, 8).await.unwrap();
        assert!(report.failure.is_none(), "{report:?}");
        assert_eq!(report.cases_run, 8);
        assert!(report.inputs_applied >= 8);
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
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let decl = counter_decl(vec![FuzzInvariant::NumericBounds {
            key: "count".to_owned(),
            min: Some(0.0),
            max: Some(2.0),
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
