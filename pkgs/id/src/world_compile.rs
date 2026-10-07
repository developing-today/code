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

use std::path::{Path, PathBuf};
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
/// Largest captured compiler output, per stream, in bytes.
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

/// The files to compile, in order; the first must be `main.roc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileSpec {
    /// `(file name, UTF-8 content)` pairs.
    pub files: Vec<(String, String)>,
    /// Seed for the compiled program's `init`.
    pub seed: u64,
}

/// How the server compiles: the platform tree and the compiler binary.
#[derive(Clone, Debug)]
pub struct Compiler {
    /// Directory containing `platform.roc` and `targets/`.
    pub platform_dir: PathBuf,
    /// The `roc` binary to run.
    pub roc_bin: String,
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
        Ok(Self {
            platform_dir,
            roc_bin,
        })
    }
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
    for (name, content) in &spec.files {
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
/// diagnostics.
///
/// # Errors
///
/// Anything outside the bounds, a compiler failure (with its diagnostics),
/// or output the sandbox refuses.
pub async fn compile(
    spec: &CompileSpec,
    compiler: &Compiler,
    timeout: Duration,
) -> Result<(Vec<u8>, String)> {
    use crate::world_session::MAX_WORLD_MODULE_BYTES;

    validate_spec(spec)?;
    let job = tempfile::tempdir().context("create compile job directory")?;
    copy_platform(&compiler.platform_dir, job.path())?;
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

    let output_path = job.path().join("out.wasm");
    let roc_bin = compiler.roc_bin.clone();
    let job_dir = job.path().to_owned();
    let output = PathBuf::from(&output_path);
    // The compiler is a whole LLVM process; run it on a blocking thread and
    // poll for the deadline so a hung run cannot block the runtime.
    let run = tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&roc_bin);
        command
            .current_dir(&job_dir)
            .args([
                "build",
                "main.roc",
                "--target=wasm32",
                "--debug",
                &format!("--output={}", output.display()),
            ])
            .env_clear()
            .env("HOME", &job_dir)
            .env("XDG_CACHE_HOME", job_dir.join("cache"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
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
        let mut stdout = child.stdout.take().context("compiler stdout")?;
        let mut stderr = child.stderr.take().context("compiler stderr")?;
        let readers = (
            std::thread::spawn(move || {
                let mut out = Vec::new();
                use std::io::Read;
                let _ = stdout.read_to_end(&mut out);
                out
            }),
            std::thread::spawn(move || {
                let mut err = Vec::new();
                use std::io::Read;
                let _ = stderr.read_to_end(&mut err);
                err
            }),
        );
        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match child.try_wait().context("wait for the compiler")? {
                Some(status) => break status,
                None => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!("compilation timed out after {}s", timeout.as_secs());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        };
        let captured_err = readers.1.join().unwrap_or_default();
        let captured_out = readers.0.join().unwrap_or_default();
        Ok((status, captured_err, captured_out))
    });
    let (status, captured_err, _captured_out) = run
        .await
        .context("compile task")?
        .map_err(|error| error.context("compilation failed"))?;
    if !status.success() {
        bail!(
            "the Roc compiler failed: {}",
            tail(&captured_err, MAX_CAPTURE_BYTES)
        );
    }

    let wasm = tokio::fs::read(&output_path)
        .await
        .context("read compiled module")?;
    ensure!(
        wasm.len() <= MAX_WORLD_MODULE_BYTES,
        "compiled module exceeds {MAX_WORLD_MODULE_BYTES} bytes"
    );
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
    Ok((wasm, tail(&captured_err, MAX_CAPTURE_BYTES)))
}

/// Copy the platform pieces a compilation needs (`platform.roc` and the
/// prebuilt `targets/`), bounded and symlink-free.
fn copy_platform(from: &Path, job: &Path) -> Result<()> {
    let staged = job.join("platform");
    std::fs::create_dir_all(&staged).context("create staged platform directory")?;
    std::fs::copy(from.join("platform.roc"), staged.join("platform.roc"))
        .context("copy platform.roc")?;
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
    ensure!(
        name.split('.').all(|part| part != "." && part != ".."),
        "file name {name:?} must stay inside the job directory"
    );
    Ok(name.to_owned())
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
        if unsafe { libc::setrlimit(resource, &limits) } != 0 {
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
        }
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
    async fn the_counter_source_compiles_installs_and_runs() {
        let Ok(roc_bin) = which_roc() else {
            eprintln!("skipping: no roc binary on PATH");
            return;
        };
        let Some(platform_dir) = default_platform_dir() else {
            eprintln!("skipping: no platform directory found");
            return;
        };
        let compiler = Compiler::new(platform_dir, roc_bin).unwrap();
        let counter = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/roc-counter/main.roc"
        ))
        .unwrap();
        eprintln!("PROBE roc={}", compiler.roc_bin);
        let (wasm, _diagnostics) = compile(
            &CompileSpec {
                files: vec![("main.roc".to_owned(), counter)],
                seed: 42,
            },
            &compiler,
            COMPILE_TIMEOUT,
        )
        .await
        .unwrap();
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
        assert_eq!(world.view(b"").unwrap(), b"count=1");
    }

    /// The roc the platform's ABI bindings were generated with; a different
    /// nightly produces a different module layout (found the hard way).
    fn which_roc() -> Result<String> {
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

    fn default_platform_dir() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/roc-world")),
            PathBuf::from("examples/roc-world"),
            PathBuf::from("../examples/roc-world"),
        ];
        candidates.into_iter().find(|dir| dir.is_dir())
    }
}
