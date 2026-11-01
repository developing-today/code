//! On-the-fly compilation of Roc world programs.
//!
//! An admin sends `.roc` files (`id world compile`); the server lays them out
//! in a private job directory beside a copy of the world platform, runs the
//! configured `roc` binary under limits, validates the result with the same
//! import-free policy as an upload, and installs it through the normal
//! journaled path. Diagnostics are returned to the caller.
//!
//! Compilation is **admin-only**: it runs a compiler on attacker-chosen
//! text, and the defense is limits (CPU, memory, file size, wall clock,
//! bounded output), not trust. The compiler itself is a trusted binary
//! configured by the operator (`--roc-bin`); packages are not fetched (the
//! world platform declares none), and the cache lives inside the job
//! directory so concurrent jobs cannot interfere.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

/// Most files one compile request may carry (including `main.roc`).
pub const MAX_COMPILE_FILES: usize = 8;
/// Largest single file, in bytes.
pub const MAX_COMPILE_FILE_BYTES: usize = 128 * 1024;
/// Largest total request, in bytes.
pub const MAX_COMPILE_TOTAL_BYTES: usize = 256 * 1024;
/// Wall-clock budget per compilation.
pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest captured compiler stderr, in bytes. Only this tail is ever
/// used for diagnostics, so the reader retains nothing more; compiler
/// stdout is not captured at all (it goes to /dev/null).
const MAX_CAPTURE_BYTES: usize = 64 * 1024;
/// Initial linear memory for Wasm apps: 129 pages, inside the sandbox's 16 MiB.
/// Roc's default (1024 pages) would trap when the module instantiates.
const WASM_MEMORY_BYTES: usize = 129 * 65536;

/// The files to compile, in order; the first must be `main.roc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileSpec {
    /// `(file name, UTF-8 content)` pairs.
    pub files: Vec<(String, String)>,
    /// Seed for the compiled program's `init`.
    pub seed: u64,
    /// Compile a native worker (`x64musl`) instead of Wasm.
    pub native: bool,
}

/// How the server compiles: the platform tree and the compiler binary.
#[derive(Clone, Debug)]
pub struct Compiler {
    /// Directory containing `platform.roc` and `targets/`.
    pub platform_dir: PathBuf,
    /// The `roc` binary to run.
    pub roc_bin: String,
    modules: Vec<String>,
}

impl Compiler {
    /// # Errors
    ///
    /// Fails when the platform directory does not look like one.
    pub fn new(platform_dir: PathBuf, roc_bin: String) -> Result<Self> {
        ensure!(
            platform_dir.join("platform.roc").is_file()
                && platform_dir.join("targets/wasm32/host.wasm").is_file(),
            "{} is not a world platform directory (need platform.roc and targets/wasm32/host.wasm)",
            platform_dir.display()
        );
        let modules = platform_modules(&platform_dir)?;
        Ok(Self {
            platform_dir,
            roc_bin,
            modules,
        })
    }
}

/// Where a compiler program runs from. A name with no slash is looked up on this
/// process's `PATH`, as a shell would. The compiler runs with a cleared environment,
/// so exec alone would search only the default path. A name that is not found is
/// kept as it is, and the spawn reports it.
fn resolve_program(program: &str) -> PathBuf {
    if program.contains('/') {
        return PathBuf::from(program);
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(program))
}

/// The platform's modules, by name: every `.roc` file but `platform.roc`.
fn platform_modules(platform_dir: &Path) -> Result<Vec<String>> {
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

/// Refuse source that imports a platform module the world does not grant.
/// Imports the platform does not provide are left to the compiler.
///
/// `granted` must be the effective list (granted and within the server and
/// world ceilings, with `*` expanded), not the raw grants.
///
/// # Errors
///
/// Names the file and the module of the first refused import.
pub fn check_imports(spec: &CompileSpec, modules: &[String], granted: &[String]) -> Result<()> {
    for (name, content) in &spec.files {
        for line in content.lines() {
            // Token-based recognition: any run of whitespace (spaces, tabs)
            // between the import keyword and the path is the same import to
            // Roc (roc fmt normalizes it), so a literal single-space prefix
            // would let ungranted imports through (#2095).
            let mut tokens = line.trim_start().split_whitespace();
            if tokens.next() != Some("import") {
                continue;
            }
            let Some(path_token) = tokens.next() else {
                continue;
            };
            let path = path_token
                .split(|c: char| matches!(c, '[' | '{' | '('))
                .next()
                .unwrap_or_default();
            for segment in path.split('.') {
                let is_platform_module = modules.iter().any(|module| module == segment);
                let is_granted = granted.iter().any(|grant| grant == "*" || grant == segment);
                ensure!(
                    !is_platform_module || is_granted,
                    "{name} imports {segment}, which this world does not allow (grant it with `id world caps --grant {segment}`; a capability ceiling may also exclude it)"
                );
            }
        }
    }
    Ok(())
}

/// Validate names, sizes and the `main.roc` requirement.
///
/// # Errors
///
/// Anything outside the bounds.
pub fn validate_spec(spec: &CompileSpec) -> Result<()> {
    ensure!(
        !spec.files.is_empty() && spec.files.len() <= MAX_COMPILE_FILES,
        "a compile request carries 1 to {MAX_COMPILE_FILES} files"
    );
    ensure!(
        spec.files
            .first()
            .is_some_and(|(name, _)| name == "main.roc"),
        "the first file must be main.roc"
    );
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for (name, content) in &spec.files {
        ensure!(
            seen.insert(name.as_str()),
            "file name {name:?} appears more than once in the compile request"
        );
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                && !name.starts_with('.'),
            "file name {name:?} is not a safe relative name"
        );
        ensure!(
            content.len() <= MAX_COMPILE_FILE_BYTES,
            "{name} exceeds {MAX_COMPILE_FILE_BYTES} bytes"
        );
        std::str::from_utf8(content.as_bytes()).context("files must be UTF-8")?;
        total += content.len();
    }
    ensure!(
        total <= MAX_COMPILE_TOTAL_BYTES,
        "the request exceeds {MAX_COMPILE_TOTAL_BYTES} bytes"
    );
    Ok(())
}

/// Compile `spec` and return the validated Wasm module plus compiler
/// diagnostics. `granted` is the effective list: the platform modules the
/// source may import, bounded by the server and world ceilings.
///
/// # Errors
///
/// Anything outside the bounds, an import the world does not grant, a compiler
/// failure (with its diagnostics), or output the sandbox refuses.
pub async fn compile(
    spec: &CompileSpec,
    compiler: &Compiler,
    granted: &[String],
    timeout: Duration,
) -> Result<(Vec<u8>, String, crate::world::ModuleProvenance)> {
    use crate::world_session::MAX_WORLD_MODULE_BYTES;

    validate_spec(spec)?;
    check_imports(spec, &compiler.modules, granted)?;
    let target = if spec.native {
        ensure!(
            compiler
                .platform_dir
                .join("targets/x64musl/libhost.a")
                .is_file(),
            "this platform has no native targets (run examples/roc-world/build-native.sh)"
        );
        "x64musl"
    } else {
        "wasm32"
    };
    let source_blake3 = digest_sources(&spec.files)?;
    let job = tempfile::tempdir().context("create compile job directory")?;
    copy_platform(&compiler.platform_dir, job.path())?;
    let platform_blake3 = digest_tree(&job.path().join("platform"))?;
    for (name, content) in &spec.files {
        let path = job.path().join(sanitize_file_name(name)?);
        tokio::fs::write(&path, content)
            .await
            .with_context(|| format!("write {name}"))?;
    }
    // The app's own platform reference is rewritten to the staged copy.
    let main_path = job.path().join("main.roc");
    let main = tokio::fs::read_to_string(&main_path)
        .await
        .context("read main.roc")?;
    let rewritten = rewrite_platform_path(&main)?;
    tokio::fs::write(&main_path, rewritten).await?;

    let output_path = job.path().join("out");
    let roc_bin = std::fs::canonicalize(resolve_program(&compiler.roc_bin))
        .context("resolve Roc compiler executable")?;
    let compiler_blake3 = digest_file(&roc_bin)?;
    let job_dir = job.path().to_owned();
    let output = PathBuf::from(&output_path);
    // The compiler is a whole LLVM process; run it on a blocking thread and
    // poll for the deadline so a hung run cannot block the runtime.
    let run = tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&roc_bin);
        let mut args = vec![
            "build".to_owned(),
            "main.roc".to_owned(),
            format!("--target={target}"),
            "--debug".to_owned(),
            format!("--output={}", output.display()),
        ];
        if target == "wasm32" {
            args.push(format!("--wasm-memory={WASM_MEMORY_BYTES}"));
        }
        let stable_arguments: Vec<String> = args
            .iter()
            .filter(|argument| !argument.starts_with("--output="))
            .cloned()
            .collect();
        command
            .current_dir(&job_dir)
            .args(&args)
            .env_clear()
            .env("HOME", &job_dir)
            .env("XDG_CACHE_HOME", job_dir.join("cache"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Safety: `pre_exec` runs the hook between fork and exec; the
            // hook only calls `setrlimit`, which is async-signal-safe.
            #[allow(unsafe_code)]
            unsafe {
                command.pre_exec(apply_rlimits);
            }
        }
        let mut child = command.spawn().context("spawn the Roc compiler")?;
        let mut stderr = child.stderr.take().context("compiler stderr")?;
        // Drain stderr on a thread while polling the deadline below. The
        // reader keeps only the last MAX_CAPTURE_BYTES: a spewing compiler
        // can neither OOM this process nor block on a full pipe, and the
        // diagnostics tail is the only part ever consumed.
        let stderr_reader =
            std::thread::spawn(move || read_capture_tail(&mut stderr, MAX_CAPTURE_BYTES));
        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait().context("wait for the compiler")? {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("compilation timed out after {}s", timeout.as_secs());
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let captured_err = stderr_reader.join().unwrap_or_default();
        Ok((status, captured_err, stable_arguments))
    });
    let (status, captured_err, arguments) = run
        .await
        .context("compile task")?
        .map_err(|error| error.context("compilation failed"))?;
    if !status.success() {
        bail!(
            "the Roc compiler failed: {}",
            tail(&captured_err, MAX_CAPTURE_BYTES)
        );
    }
    let provenance =
        crate::world::ModuleProvenance::SourceBuild(crate::world::SourceBuildProvenance {
            schema_version: 1,
            compiler_blake3,
            platform_blake3,
            source_blake3,
            target: target.to_owned(),
            arguments,
        });

    let wasm = tokio::fs::read(&output_path)
        .await
        .context("read compiled module")?;
    ensure!(
        wasm.len() <= MAX_WORLD_MODULE_BYTES,
        "compiled module exceeds {MAX_WORLD_MODULE_BYTES} bytes"
    );
    if spec.native {
        // The install loads the worker under its sandbox, which is where a
        // bad executable is refused.
        ensure!(
            crate::world_native::is_native_module(&wasm),
            "the compiler did not produce a native executable"
        );
        return Ok((wasm, tail(&captured_err, MAX_CAPTURE_BYTES), provenance));
    }
    // Same policy as an upload: import-free and allowlisted.
    #[cfg(feature = "sandbox")]
    crate::sandbox::Sandbox::compile(
        &wasm,
        crate::sandbox::SandboxLimits {
            module_bytes: MAX_WORLD_MODULE_BYTES,
            ..crate::sandbox::SandboxLimits::default()
        },
    )
    .context("the compiled module is not a valid world module")?;
    Ok((wasm, tail(&captured_err, MAX_CAPTURE_BYTES), provenance))
}

fn digest_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"roc-compiler-binary-v1\0");
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        use std::io::Read as _;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn digest_tree(root: &Path) -> Result<String> {
    fn visit(root: &Path, dir: &Path, hasher: &mut blake3::Hasher) -> Result<()> {
        let mut entries = std::fs::read_dir(dir)
            .with_context(|| format!("read {}", dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            ensure!(
                !entry.file_type()?.is_symlink(),
                "no symlinks in the platform"
            );
            let path = entry.path();
            if path.is_dir() {
                let relative = path.strip_prefix(root)?;
                let name = relative.to_string_lossy();
                let name_bytes = name.as_bytes();
                let name_len = u64::try_from(name_bytes.len()).context("platform path length")?;
                hasher.update(b"D");
                hasher.update(&name_len.to_le_bytes());
                hasher.update(name_bytes);
                visit(root, &path, hasher)?;
                continue;
            }
            let relative = path.strip_prefix(root)?;
            let name = relative.to_string_lossy();
            let name_bytes = name.as_bytes();
            let name_len = u64::try_from(name_bytes.len()).context("platform path length")?;
            hasher.update(b"F");
            hasher.update(&name_len.to_le_bytes());
            hasher.update(name_bytes);
            let mut file = std::fs::File::open(&path)?;
            let file_len = file.metadata()?.len();
            hasher.update(&file_len.to_le_bytes());
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                use std::io::Read as _;
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        }
        Ok(())
    }

    let mut hasher = blake3::Hasher::new();
    visit(root, root, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn digest_sources(files: &[(String, String)]) -> Result<String> {
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"roc-source-files-v1\0");
    for (name, content) in sorted {
        let name = name.as_bytes();
        let content = content.as_bytes();
        let name_len = u64::try_from(name.len()).context("source filename length")?;
        let content_len = u64::try_from(content.len()).context("source file length")?;
        hasher.update(&name_len.to_le_bytes());
        hasher.update(name);
        hasher.update(&content_len.to_le_bytes());
        hasher.update(content);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Copy the platform pieces a compilation needs (its `.roc` modules, including
/// `platform.roc`, and the prebuilt `targets/`), bounded and symlink-free.
fn copy_platform(from: &Path, job: &Path) -> Result<()> {
    let staged = job.join("platform");
    std::fs::create_dir_all(&staged).context("create staged platform directory")?;
    for entry in std::fs::read_dir(from).context("read platform directory")? {
        let entry = entry?;
        let path = entry.path();
        ensure!(
            !entry.file_type()?.is_symlink(),
            "no symlinks in the platform"
        );
        if path.extension().is_some_and(|ext| ext == "roc") && path.is_file() {
            std::fs::copy(&path, staged.join(entry.file_name()))
                .with_context(|| format!("copy {}", path.display()))?;
        }
    }
    copy_dir(&from.join("targets"), &staged.join("targets"), 0)?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path, depth: usize) -> Result<()> {
    ensure!(depth <= 4, "platform targets tree is unexpectedly deep");
    std::fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let path = entry.path();
        ensure!(
            !entry.file_type()?.is_symlink(),
            "no symlinks in the platform"
        );
        if path.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()), depth + 1)?;
        } else {
            std::fs::copy(&path, to.join(entry.file_name()))
                .with_context(|| format!("copy {}", path.display()))?;
        }
    }
    Ok(())
}

/// Point the app at the staged platform: the first `pf: platform "..."` gets
/// `platform/platform.roc`.
fn rewrite_platform_path(main: &str) -> Result<String> {
    let needle = "pf: platform \"";
    let start = main
        .find(needle)
        .context("main.roc must declare its platform with `pf: platform \"...\"`")?;
    let after = start + needle.len();
    let end = main[after..]
        .find('"')
        .context("the platform path is unterminated")?;
    Ok(format!(
        "{}platform/platform.roc{}",
        &main[..after],
        &main[after + end..]
    ))
}

fn sanitize_file_name(name: &str) -> Result<String> {
    // The name must be exactly one normal path component: no parent
    // references, no separators, no root. (Splitting on '.' cannot
    // detect ".." — its parts are maximal non-dot runs and never
    // contain a dot.)
    let mut components = Path::new(name).components();
    ensure!(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
        "file name {name:?} must stay inside the job directory"
    );
    Ok(name.to_owned())
}

/// Read `reader` to EOF, retaining only the last `cap` bytes.
///
/// Draining never stops at the cap: the writer (a compiler child on a
/// pipe) must never block on a full pipe, and the retained memory stays
/// bounded at `cap` bytes plus one read chunk no matter how much the
/// writer emits. Interrupted reads are retried (as `read_to_end` does);
/// any other read error ends the capture with what was retained.
fn read_capture_tail(reader: &mut impl std::io::Read, cap: usize) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                retained.extend_from_slice(&chunk[..read]);
                if retained.len() > cap {
                    retained.drain(..retained.len() - cap);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    retained
}

fn tail(bytes: &[u8], max: usize) -> String {
    let bytes = if bytes.len() > max {
        &bytes[bytes.len() - max..]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).into_owned()
}

/// CPU, memory, file-size and process limits for the compiler child.
#[cfg(unix)]
/// Safety: runs between fork and exec; only async-signal-safe calls
/// (`setrlimit`) are made.
fn apply_rlimits() -> std::io::Result<()> {
    let cap = |resource: u32, limit: libc::rlim_t| {
        let limits = libc::rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        // Safety: between fork and exec; `setrlimit` is async-signal-safe.
        #[allow(unsafe_code)]
        if unsafe { libc::setrlimit(resource, &raw const limits) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    // No process limit: RLIMIT_NPROC counts the user's whole process table
    // (threads included), so a low soft limit would starve the compiler's
    // worker threads for reasons unrelated to this job.
    cap(libc::RLIMIT_CPU, 120)?;
    cap(libc::RLIMIT_AS, 8 * 1024 * 1024 * 1024)?;
    cap(libc::RLIMIT_FSIZE, 256 * 1024 * 1024)?;
    Ok(())
}

/// Locate the Roc compiler binary for world builds: `$ID_ROC_BIN`, then the
/// pinned nightly the platform's ABI bindings were generated with, then a
/// `roc` on `PATH` that answers `roc version`.
///
/// The exact nightly matters: a different one produces a different module
/// layout (found the hard way). `id world doctor` and the compile path agree
/// on the toolchain by both going through this function.
pub fn locate_roc() -> Result<String> {
    if let Ok(path) = std::env::var("ID_ROC_BIN") {
        return Ok(path);
    }
    let tag = "nightly-2026-10-04-130536d";
    let version = tag.trim_start_matches("nightly-");
    for home in [
        std::env::var("HOME").unwrap_or_default(),
        "/home/user".to_owned(),
    ] {
        let path = format!(
            "{home}/.local/share/plaza-tools/roc/{tag}/roc_nightly-linux_x86_64-{version}/roc"
        );
        if Path::new(&path).is_file() {
            return Ok(path);
        }
    }
    let output = std::process::Command::new("roc").arg("version").output();
    match output {
        Ok(output) if output.status.success() => Ok("roc".to_owned()),
        _ => bail!("no roc binary"),
    }
}

#[cfg(all(test, feature = "sandbox"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn spec(files: Vec<(&str, &str)>) -> CompileSpec {
        CompileSpec {
            files: files
                .into_iter()
                .map(|(name, content)| (name.to_owned(), content.to_owned()))
                .collect(),
            seed: 1,
            native: false,
        }
    }

    /// A reader that generates `total` bytes of a deterministic pattern
    /// without storing them, counting how many bytes were consumed so a
    /// test can prove the capture drained it to EOF.
    struct Spew {
        produced: usize,
        total: usize,
    }

    impl std::io::Read for Spew {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = buffer.len().min(self.total - self.produced);
            for slot in &mut buffer[..count] {
                *slot = (self.produced % 251) as u8;
                self.produced += 1;
            }
            Ok(count)
        }
    }

    #[test]
    fn capture_tail_stays_bounded_when_the_compiler_spews() {
        // >>64 KiB of stderr (the triage probe used a 200 MiB spew through
        // a real child pipe): the retained capture must stay at the cap,
        // hold exactly the LAST cap bytes, and the reader must have
        // drained the stream to EOF so the child never blocks.
        let total = 4 * 1024 * 1024;
        let mut spew = Spew { produced: 0, total };
        let captured = read_capture_tail(&mut spew, MAX_CAPTURE_BYTES);
        assert_eq!(spew.produced, total, "the stream must be drained to EOF");
        assert_eq!(captured.len(), MAX_CAPTURE_BYTES);
        let expected: Vec<u8> = ((total - MAX_CAPTURE_BYTES)..total)
            .map(|position| (position % 251) as u8)
            .collect();
        assert_eq!(captured, expected, "the retained bytes are the tail");
        // And the diagnostics path renders exactly that tail (lossy UTF-8,
        // so compare against the lossy rendering, not a byte length).
        assert_eq!(
            tail(&captured, MAX_CAPTURE_BYTES),
            String::from_utf8_lossy(&expected).into_owned()
        );
    }

    #[test]
    fn capture_tail_keeps_small_output_whole() {
        let mut small = std::io::Cursor::new(b"error: nope\n".to_vec());
        assert_eq!(
            read_capture_tail(&mut small, MAX_CAPTURE_BYTES),
            b"error: nope\n"
        );
        let mut empty = std::io::Cursor::new(Vec::new());
        assert!(read_capture_tail(&mut empty, MAX_CAPTURE_BYTES).is_empty());
    }

    /// A reader whose first read fails with `Interrupted`, then yields
    /// its bytes: the capture must retry, not stop draining.
    struct InterruptedOnce {
        interrupted: bool,
        inner: std::io::Cursor<Vec<u8>>,
    }

    impl std::io::Read for InterruptedOnce {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
            }
            self.inner.read(buffer)
        }
    }

    #[test]
    fn capture_tail_retries_interrupted_reads() {
        let mut reader = InterruptedOnce {
            interrupted: false,
            inner: std::io::Cursor::new(b"diagnostics after EINTR\n".to_vec()),
        };
        assert_eq!(
            read_capture_tail(&mut reader, MAX_CAPTURE_BYTES),
            b"diagnostics after EINTR\n"
        );
    }

    #[test]
    fn specs_are_bounded_and_need_main_first() {
        validate_spec(&spec(vec![("main.roc", "app [x] {}")])).unwrap();
        assert!(validate_spec(&spec(vec![("lib.roc", "x")])).is_err());
        assert!(validate_spec(&spec(vec![("main.roc", "x"), ("../evil.roc", "y")])).is_err());

        assert!(validate_spec(&spec(vec![("main.roc", "x"), (".hidden", "y")])).is_err());
        let too_big = spec(vec![("main.roc", &"x".repeat(MAX_COMPILE_FILE_BYTES + 1))]);
        assert!(validate_spec(&too_big).is_err());
        let mut many = vec![("main.roc", "x")];
        for i in 0..MAX_COMPILE_FILES {
            many.push((Box::leak(format!("f{i}.roc").into_boxed_str()), "y"));
        }
        assert!(validate_spec(&spec(many)).is_err());
    }

    #[test]
    fn sanitize_file_name_refuses_traversal() {
        for name in [
            "..",
            ".",
            "",
            "../evil.roc",
            "../../evil.roc",
            "a/../../evil.roc",
            "sub/Foo.roc",
            "/etc/passwd",
        ] {
            assert!(
                sanitize_file_name(name).is_err(),
                "sanitize_file_name({name:?}) must refuse"
            );
        }
        for name in ["main.roc", "Helper.roc", "a..b.roc", "foo-bar_baz.roc"] {
            assert_eq!(sanitize_file_name(name).unwrap(), name);
        }
    }

    #[test]
    fn duplicate_file_names_are_refused() {
        // A duplicated helper: staging writes both to the same job path, so
        // the first would be silently overwritten — and digest_sources would
        // still hash it into the recorded provenance.
        let duplicate = spec(vec![
            ("main.roc", "app"),
            ("Helper.roc", "first"),
            ("Helper.roc", "second"),
        ]);
        let error = validate_spec(&duplicate).unwrap_err().to_string();
        assert!(error.contains("Helper.roc"), "{error}");
        assert!(error.contains("more than once"), "{error}");

        // A duplicated main.roc: files[0] satisfies the main.roc rule, but
        // the main.roc read back from the job directory would be the second.
        let duplicate_main = spec(vec![
            ("main.roc", "app"),
            ("Helper.roc", "helper"),
            ("main.roc", "other"),
        ]);
        assert!(validate_spec(&duplicate_main).is_err());

        // Control: unique names still validate.
        let unique = spec(vec![("main.roc", "app"), ("Helper.roc", "helper")]);
        assert!(validate_spec(&unique).is_ok());
    }

    #[test]
    fn source_digest_is_independent_of_file_order_but_tracks_names_and_contents() {
        let first = vec![
            ("main.roc".to_owned(), "app [] {}".to_owned()),
            ("lib.roc".to_owned(), "value = 1".to_owned()),
        ];
        let reversed = vec![first[1].clone(), first[0].clone()];
        assert_eq!(
            digest_sources(&first).unwrap(),
            digest_sources(&reversed).unwrap()
        );

        let changed = vec![
            first[0].clone(),
            ("lib.roc".to_owned(), "value = 2".to_owned()),
        ];
        assert_ne!(
            digest_sources(&first).unwrap(),
            digest_sources(&changed).unwrap()
        );
    }

    #[test]
    fn platform_digest_is_stable_and_includes_relative_names() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("targets/wasm32")).unwrap();
        std::fs::write(root.path().join("platform.roc"), "platform").unwrap();
        std::fs::write(root.path().join("targets/wasm32/host.wasm"), b"host").unwrap();

        let before = digest_tree(root.path()).unwrap();
        assert_eq!(before, digest_tree(root.path()).unwrap());
        std::fs::create_dir(root.path().join("empty")).unwrap();
        assert_ne!(before, digest_tree(root.path()).unwrap());
        std::fs::write(root.path().join("targets/wasm32/host.wasm"), b"other").unwrap();
        assert_ne!(before, digest_tree(root.path()).unwrap());
    }

    #[test]
    fn the_platform_path_is_rewritten_to_the_staged_copy() {
        let main = "app [program] { pf: platform \"../roc-world/platform.roc\" }\n";
        assert_eq!(
            rewrite_platform_path(main).unwrap(),
            "app [program] { pf: platform \"platform/platform.roc\" }\n"
        );
        assert!(rewrite_platform_path("app [x] {}").is_err());
        assert!(rewrite_platform_path("pf: platform \"unterminated").is_err());
    }

    #[tokio::test]
    async fn the_capability_helper_issues_matching_requests() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let app = r#"app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocCap
import pf.RocJson

Model : { session : RocCap, request : Str }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |seed| {
	issued = RocCap.issue(RocCap.empty, "chat.say")
	document = RocCap.chat_request(issued.id, seed.to_str())
	first = RocCap.attach(issued.session, issued.id, document)
	second = RocCap.issue(first, "chat.history")
	history = RocCap.history_request(second.id, 5)
	session = RocCap.attach(second.session, second.id, history)
	encoded = match RocJson.encode(document) {
		Ok(text) => text
		Err(_) => ""
	}
	{ session: session, request: encoded }
}

update : Model, U64, Str -> Model
update = |model, _pid, _ev| model

view : Model -> Str
view = |model| model.request

layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |model| {
	encoded = List.map(RocCap.requests(model.session), |document| match RocJson.encode(document) {
		Ok(text) => text
		Err(_) => ""
	})
	"{\"requests\":[${Str.join_with(encoded, ",")}]}"
}

snapshot : Model -> Str
snapshot = |model| model.request

restore : Str -> Model
restore = |text| { session: RocCap.empty, request: text }
"#;
        let (wasm, _diagnostics, _provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), app.to_owned())],
                seed: 7,
                native: false,
            },
            &compiler,
            &["RocCap".to_owned(), "RocJson".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(7).unwrap();
        let view = String::from_utf8(world.view().unwrap()).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&view).unwrap();
        assert_eq!(decoded["id"], "r0");
        assert_eq!(decoded["cap"], "chat.say");
        assert_eq!(decoded["args"]["text"], "7");
        let wants = crate::world::WorldProgram::wants(&mut world)
            .unwrap()
            .unwrap();
        let decoded_wants: serde_json::Value = serde_json::from_str(&wants).unwrap();
        assert_eq!(decoded_wants["requests"][0]["id"], "r0");
        assert_eq!(decoded_wants["requests"][0]["cap"], "chat.say");
        assert_eq!(decoded_wants["requests"][1]["id"], "r1");
        assert_eq!(decoded_wants["requests"][1]["cap"], "chat.history");
        assert_eq!(decoded_wants["requests"][1]["args"]["count"], 5);
    }

    #[tokio::test]
    async fn the_schedule_helpers_build_schedule_requests() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let app = r#"app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocCap
import pf.RocJson

Model : { documents : List(RocJson) }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |seed| {
	scheduled = RocCap.schedule_at_request("r0", "nap", seed * 1000)
	cancelled = RocCap.schedule_cancel_request("r1", "nap")
	{ documents: [scheduled, cancelled] }
}

update : Model, U64, Str -> Model
update = |model, _pid, _ev| model

view : Model -> Str
view = |_model| ""

layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |model| {
	encoded = List.map(model.documents, |document| match RocJson.encode(document) {
		Ok(text) => text
		Err(_) => ""
	})
	"{\"requests\":[${Str.join_with(encoded, ",")}]}"
}

snapshot : Model -> Str
snapshot = |_model| ""

restore : Str -> Model
restore = |_text| { documents: [] }
"#;
        let (wasm, _diagnostics, _provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), app.to_owned())],
                seed: 7,
                native: false,
            },
            &compiler,
            &["RocCap".to_owned(), "RocJson".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(7).unwrap();
        let wants = crate::world::WorldProgram::wants(&mut world)
            .unwrap()
            .unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&wants).unwrap();
        assert_eq!(decoded["requests"][0]["id"], "r0");
        assert_eq!(decoded["requests"][0]["cap"], "schedule.at");
        assert_eq!(decoded["requests"][0]["args"]["id"], "nap");
        assert_eq!(decoded["requests"][0]["args"]["at"], 7000);
        assert_eq!(decoded["requests"][1]["id"], "r1");
        assert_eq!(decoded["requests"][1]["cap"], "schedule.cancel");
        assert_eq!(decoded["requests"][1]["args"]["id"], "nap");
    }

    #[test]
    fn the_platform_modules_are_staged_with_the_platform() {
        let from = tempfile::tempdir().unwrap();
        let job = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("platform.roc"), "x").unwrap();
        std::fs::write(from.path().join("Screen.roc"), "y").unwrap();
        std::fs::write(from.path().join("notes.txt"), "z").unwrap();
        std::fs::create_dir_all(from.path().join("targets")).unwrap();
        copy_platform(from.path(), job.path()).unwrap();
        assert!(job.path().join("platform/platform.roc").is_file());
        assert!(job.path().join("platform/Screen.roc").is_file());
        assert!(!job.path().join("platform/notes.txt").exists());
    }

    #[tokio::test]
    async fn screen_widgets_fill_and_rule() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let app = r#"app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Screen

Model : { n : U64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |seed| { n: seed }

update : Model, U64, Str -> Model
update = |model, _pid, _ev| model

view : Model -> Str
view = |model| {
    blank = Screen.new(6, 3)
    filled = Screen.fill(blank, 0, 0, 3, 2, "x", { fg: 0, bg: 0, bold: Bool.False })
    ruled = Screen.rule(filled, 0, 2, 6, "-", { fg: 0, bg: 0, bold: Bool.False })
    written = Screen.put_text(ruled, 4, 1, model.n.to_str(), { fg: 0, bg: 0, bold: Bool.False })
    Screen.render_plain(written)
}

layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| model.n.to_str()

restore : Str -> Model
restore = |text| match U64.from_str(text) {
    Ok(n) => { n: n }
    Err(_) => { n: 0 }
}
"#;
        let (wasm, _diagnostics, _provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), app.to_owned())],
                seed: 7,
                native: false,
            },
            &compiler,
            &["Screen".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(7).unwrap();
        let view = String::from_utf8(world.view().unwrap()).unwrap();
        assert_eq!(view, "xxx   \nxxx 7 \n------");
    }

    #[tokio::test]
    async fn text_helpers_trim_case_split_and_prefix() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let app = r#"app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocText

Model : { n : U64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |seed| { n: seed }

update : Model, U64, Str -> Model
update = |model, _pid, _ev| model

view : Model -> Str
view = |model| {
    trimmed = RocText.trim("  AbC\t\n")
    lowered = RocText.lowercase(trimmed)
    prefix = if model.n > 0 "hello" else "nope"
    prefixed = RocText.starts_with("hello world", prefix)
    prefixed_text = if prefixed "yes" else "no"
    dropped = RocText.drop_prefix("cmd:go", "cmd:")
    missing = RocText.drop_prefix("keep", "cmd:")
    joined = RocText.join_lines(RocText.lines("a\nb\n"))
    needle = if model.n > 0 "lo wo" else "zz"
    found = RocText.contains("hello world", needle)
    found_text = if found "yes" else "no"
    "t=${trimmed}|l=${lowered}|p=${prefixed_text}|d=${dropped}|m=${missing}|j=${joined}|c=${found_text}"
}

layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| model.n.to_str()

restore : Str -> Model
restore = |text| match U64.from_str(text) {
    Ok(n) => { n: n }
    Err(_) => { n: 0 }
}
"#;
        let (wasm, _diagnostics, _provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), app.to_owned())],
                seed: 7,
                native: false,
            },
            &compiler,
            &["RocText".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(7).unwrap();
        let view = String::from_utf8(world.view().unwrap()).unwrap();
        assert_eq!(view, "t=AbC|l=abc|p=yes|d=go|m=keep|j=a\nb\n|c=yes");
    }

    #[tokio::test]
    async fn the_counter_source_compiles_installs_and_runs() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let counter = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-counter/main.roc"
        ))
        .unwrap();
        eprintln!("PROBE roc={}", compiler.roc_bin);
        let (wasm, _diagnostics, provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), counter)],
                seed: 42,
                native: false,
            },
            &compiler,
            &["Screen".to_owned(), "RocText".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        let crate::world::ModuleProvenance::SourceBuild(build) = provenance else {
            panic!("source compilation must return build provenance");
        };
        assert_eq!(build.schema_version, 1);
        assert_eq!(build.target, "wasm32");
        assert_eq!(build.arguments[0], "build");
        assert_eq!(build.compiler_blake3.len(), 64);
        assert_eq!(build.platform_blake3.len(), 64);
        assert_eq!(build.source_blake3.len(), 64);
        // The compiled module behaves like the checked-in one.
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(42).unwrap();
        crate::world::WorldProgram::update(
            &mut world,
            &crate::world::WorldEvent {
                sequence: 1,
                participant_id: 1,
                kind: crate::world::WorldEventKind::Input(b"inc".to_vec()),
            },
        )
        .unwrap();
        assert!(
            String::from_utf8(world.view().unwrap())
                .unwrap()
                .contains("count=1")
        );
    }

    #[tokio::test]
    async fn apps_can_import_the_platform_screen_module() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let app = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-screen/main.roc"
        ))
        .unwrap();
        let (wasm, _diagnostics, provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), app)],
                seed: 7,
                native: false,
            },
            &compiler,
            &["Screen".to_owned(), "Key".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        assert!(matches!(
            provenance,
            crate::world::ModuleProvenance::SourceBuild(_)
        ));
        let runner =
            crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let mut world = runner.instantiate(7).unwrap();
        crate::world::WorldProgram::update(
            &mut world,
            &crate::world::WorldEvent {
                sequence: 1,
                participant_id: 1,
                kind: crate::world::WorldEventKind::Input(b"+".to_vec()),
            },
        )
        .unwrap();
        let frame = String::from_utf8(world.view().unwrap()).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) =
            crate::world_presentation::parse_surface(&frame).unwrap()
        else {
            panic!("Roc screen example must return a structured screen");
        };
        assert_eq!((screen.width, screen.height), (20, 3));
        let rows: Vec<String> = screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect();
        assert!(rows[0].starts_with("+-counter"));
        assert!(rows[1].starts_with("| count=1"));
        assert_eq!(rows[2], format!("+{}+", "-".repeat(18)));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_counter_source_compiles_natively_and_runs_in_the_worker() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        if !platform_dir.join("targets/x64musl/libhost.a").is_file() {
            eprintln!("skipping: the platform has no native targets");
            return;
        }
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let counter = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-counter/main.roc"
        ))
        .unwrap();
        let (native, _diagnostics, provenance) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), counter)],
                seed: 42,
                native: true,
            },
            &compiler,
            &["Screen".to_owned(), "RocText".to_owned()],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
        assert!(matches!(
            provenance,
            crate::world::ModuleProvenance::SourceBuild(_)
        ));
        assert!(crate::world_native::is_native_module(&native));
        let job = tempfile::tempdir().unwrap();
        let path = job.path().join("counter");
        std::fs::write(&path, &native).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let mut world = crate::world_native::NativeProgram::new(
            &path,
            42,
            crate::world_native::NativeLimits::default(),
            None,
        )
        .unwrap();
        crate::world::WorldProgram::update(
            &mut world,
            &crate::world::WorldEvent {
                sequence: 1,
                participant_id: 1,
                kind: crate::world::WorldEventKind::Input(b"inc".to_vec()),
            },
        )
        .unwrap();
        let view = crate::world::WorldProgram::view(
            &mut world,
            &crate::world::Participant {
                id: 1,
                display_name: "ann".to_owned(),
            },
        )
        .unwrap();
        assert!(view.as_deref().is_some_and(|view| view.contains("count=1")));
    }

    #[test]
    fn an_ungranted_platform_import_is_refused_by_name() {
        let modules = vec!["Key".to_owned(), "Screen".to_owned()];
        let app = spec(vec![("main.roc", "import pf.Screen\napp [x] {}")]);
        let refused = check_imports(&app, &modules, &[]).unwrap_err().to_string();
        assert!(refused.contains("main.roc imports Screen"), "{refused}");
        assert!(check_imports(&app, &modules, &["Screen".to_owned()]).is_ok());
        assert!(check_imports(&app, &modules, &["*".to_owned()]).is_ok());
        assert!(check_imports(&app, &modules, &["Key".to_owned()]).is_err());
    }

    #[test]
    fn imports_outside_the_platform_and_comments_pass_the_gate() {
        let modules = vec!["Screen".to_owned()];
        let app = spec(vec![
            (
                "main.roc",
                "# import pf.Screen\nimport pf.Other exposing [x]\nimport Helper\n",
            ),
            ("Helper.roc", "module []\n"),
        ]);
        assert!(check_imports(&app, &modules, &[]).is_ok());
    }

    #[test]
    fn an_ungranted_platform_import_with_extra_whitespace_is_refused_by_name() {
        let modules = vec!["Key".to_owned(), "Screen".to_owned()];
        // Roc accepts every spelling below and roc fmt normalizes them to
        // the single-space form, so the gate must recognize them all (#2095).
        for spelling in [
            "import  pf.Screen",
            "import\tpf.Screen",
            "import \t pf.Screen",
            "  import  pf.Screen",
        ] {
            let app = spec(vec![("main.roc", &format!("{spelling}\napp [x] {{}}"))]);
            let refused = check_imports(&app, &modules, &[]).unwrap_err().to_string();
            assert!(refused.contains("main.roc imports Screen"), "{spelling}: {refused}");
            assert!(check_imports(&app, &modules, &["Screen".to_owned()]).is_ok(), "{spelling}");
            assert!(check_imports(&app, &modules, &["*".to_owned()]).is_ok(), "{spelling}");
        }
    }

    #[test]
    fn a_sibling_file_cannot_smuggle_an_ungranted_import_with_extra_whitespace() {
        let modules = vec!["Key".to_owned()];
        for spelling in ["import  pf.Key", "import\tpf.Key"] {
            let app = spec(vec![
                ("main.roc", "app [x] {}"),
                ("Helper.roc", &format!("{spelling}\n")),
            ]);
            assert!(check_imports(&app, &modules, &[]).is_err(), "{spelling}");
        }
    }

    #[test]
    fn a_sibling_file_cannot_smuggle_an_ungranted_import() {
        let modules = vec!["Key".to_owned()];
        let app = spec(vec![
            ("main.roc", "app [x] {}"),
            ("Helper.roc", "import pf.Key\n"),
        ]);
        assert!(check_imports(&app, &modules, &[]).is_err());
    }

    #[tokio::test]
    async fn an_ungranted_import_is_refused_before_the_compiler_runs() {
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!(
                "skipping: no roc-world platform with targets/wasm32/host.wasm (run examples/roc-world/build-host.sh)"
            );
            return;
        };
        let compiler = Compiler::new(platform_dir, "/nonexistent/roc".to_owned()).unwrap();
        let app = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-screen/main.roc"
        ))
        .unwrap();
        let error = compile(
            &spec(vec![("main.roc", &app)]),
            &compiler,
            &[],
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("imports Screen"), "{error:#}");
    }

    /// The roc the platform's ABI bindings were generated with; a different
    /// nightly produces a different module layout (found the hard way).
    fn which_roc() -> Result<String> {
        crate::world_compile::locate_roc()
    }

    /// The first candidate that is a complete world platform. The built
    /// `host.wasm` is required too, so the compile tests skip on a checkout
    /// where `examples/roc-world/build-host.sh` has not run.
    fn first_platform_with_host(candidates: &[PathBuf]) -> Option<PathBuf> {
        candidates
            .iter()
            .find(|dir| dir.join("targets/wasm32/host.wasm").is_file())
            .cloned()
    }

    fn default_platform_dir() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/roc-world")),
            PathBuf::from("examples/roc-world"),
            PathBuf::from("../examples/roc-world"),
        ];
        first_platform_with_host(&candidates)
    }

    #[test]
    fn the_default_platform_needs_a_built_host_wasm() {
        let root = tempfile::tempdir().unwrap();
        let bare = root.path().join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("platform.roc"), "x").unwrap();
        let built = root.path().join("built");
        std::fs::create_dir_all(built.join("targets/wasm32")).unwrap();
        std::fs::write(built.join("platform.roc"), "x").unwrap();
        std::fs::write(built.join("targets/wasm32/host.wasm"), b"\0asm").unwrap();

        assert_eq!(
            first_platform_with_host(&[bare.clone(), built.clone()]),
            Some(built)
        );
        assert_eq!(first_platform_with_host(&[bare]), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_bare_compiler_name_resolves_to_its_file_on_this_path() {
        // `sh` is on the PATH the tests run with, wherever that PATH points.
        let shell = resolve_program("sh");
        assert!(
            shell.is_absolute() && shell.is_file(),
            "{}",
            shell.display()
        );
        assert_eq!(resolve_program("/opt/roc"), PathBuf::from("/opt/roc"));
        assert_eq!(
            resolve_program("no-such-compiler"),
            PathBuf::from("no-such-compiler")
        );
    }

    #[test]
    fn a_ceiling_bounds_the_effective_list_the_gate_sees() {
        use crate::world_caps::CapLedger;
        use std::collections::BTreeSet;

        let modules = vec!["Key".to_owned(), "Screen".to_owned()];
        let app = spec(vec![("main.roc", "import pf.Screen\napp [x] {}")]);
        // `*` is granted, but the ceiling admits only Key: the gate sees Key alone.
        let mut ledger = CapLedger::new(BTreeSet::from(["*".to_owned()]), false);
        ledger.set_allowed(BTreeSet::from(["Key".to_owned()]));
        let effective: Vec<String> = ledger.effective().into_iter().collect();
        let refused = check_imports(&app, &modules, &effective)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("main.roc imports Screen"), "{refused}");

        ledger.set_allowed(BTreeSet::from(["Key".to_owned(), "Screen".to_owned()]));
        let effective: Vec<String> = ledger.effective().into_iter().collect();
        assert!(check_imports(&app, &modules, &effective).is_ok());
    }

    #[tokio::test]
    async fn an_import_outside_the_effective_list_is_refused_before_the_compiler_runs() {
        // A platform directory that passes `Compiler::new`, with no roc binary.
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("platform.roc"), "x").unwrap();
        std::fs::create_dir_all(root.path().join("targets/wasm32")).unwrap();
        std::fs::write(root.path().join("targets/wasm32/host.wasm"), b"\0asm").unwrap();
        std::fs::write(root.path().join("Screen.roc"), "module []\n").unwrap();
        let compiler =
            Compiler::new(root.path().to_path_buf(), "/nonexistent/roc".to_owned()).unwrap();
        let app = spec(vec![("main.roc", "import pf.Screen\napp [x] {}")]);
        let error = compile(&app, &compiler, &["Key".to_owned()], COMPILE_TIMEOUT)
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("main.roc imports Screen"),
            "{error:#}"
        );
    }
}
