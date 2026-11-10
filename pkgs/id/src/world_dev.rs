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
//! Later phases (watch/debounce + F12.9 hot-swap, fixture promotion
//! gates, deterministic fuzz) build on [`DevScratchWorld`] and
//! [`discover_sources`]; they are deliberately not stubbed here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::sandbox::{Sandbox, SandboxLimits};
use crate::world::{JoinCapability, WorldCore, WorldHandle, WorldLimits, WorldScopes};
use crate::world_compile::{COMPILE_TIMEOUT, CompileSpec, Compiler, validate_spec};

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
    /// The view before any input was sent.
    pub initial_view: Option<String>,
    /// Inputs applied, in order, each with its resulting view.
    pub steps: Vec<DevStep>,
    /// Records sequence reached by the end of the session.
    pub records_sequence: u64,
}

/// Run one Phase 1 dev session: discover, compile, launch, apply
/// `inputs`, and report. The scratch world is dropped (discarded)
/// before this returns.
///
/// # Errors
///
/// Discovery, platform/compiler setup, compilation, or a guest call
/// fails; the error names the failing stage. A failed compile leaves
/// nothing running — there is no last-good version in a one-shot
/// session, and no hosted world is ever touched.
pub async fn run_dev_once(
    dir: &Path,
    platform: Option<&Path>,
    roc: Option<&str>,
    seed: u64,
    inputs: &[String],
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
    };
    // The scratch world ends with the session: dropping `world` here
    // discards its state, which is the Phase 1 lifecycle guarantee.
    drop(world);
    Ok(report)
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
}
