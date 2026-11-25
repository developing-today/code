use super::*;

/// Wall-clock bound for the compiler version probe (#1069). The probe
/// is informational provenance only, so it gets a small slice of the
/// compilation budget; a compiler that cannot state its version within
/// the bound is recorded as an empty version.
pub(super) const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ask the compiler for its self-reported version, bounded in time and
/// captured output. Runs on a blocking thread (called from the compile
/// job's `spawn_blocking` closure): the child is polled against
/// `timeout` and killed past it, and its stdout is drained nonblocking
/// into a tail-bounded buffer, so neither a hanging compiler nor a
/// grandchild inheriting the stdout pipe can block the probe — there
/// is no reader thread to join and no EOF to wait for. Any failure —
/// spawn, non-zero exit, timeout — yields an empty string; the caller
/// pins artifact identity by digest regardless.
pub(super) fn probe_compiler_version(roc_bin: &Path, timeout: Duration) -> String {
    probe_compiler_version_outcome(roc_bin, timeout).into_version()
}

/// Which way a version probe ended (#8173). [`probe_compiler_version`]
/// collapses every failure to an empty string ON PURPOSE — the probe
/// is informational provenance and a compile must not fail over it —
/// but that collapse also made the load flake in
/// `version_probe_records_the_reported_version` undiagnosable: the
/// failing assertion could not say whether the fake compiler failed
/// to spawn, was killed at the deadline, or exited non-zero. The
/// detailed outcome keeps the branches apart for tests and
/// diagnostics; the compile path still records only the string.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum VersionProbeOutcome {
    /// The compiler exited successfully; its trimmed reported version.
    Reported(String),
    /// The compiler process could not be spawned (or its stdout pipe
    /// was unavailable): under load this is the resource-starvation
    /// branch (fork/exec pressure), not a compiler answer.
    SpawnFailed,
    /// The compiler was still running at the probe deadline and was
    /// killed: under load this is the scheduling-starvation branch.
    TimedOut,
    /// The compiler ran and exited non-zero (its exit code, when the
    /// platform reports one). Deterministic for a given compiler —
    /// never a load flake.
    ExitedNonZero(Option<i32>),
    /// Polling the child failed; the probe cannot say more.
    WaitFailed,
}

impl VersionProbeOutcome {
    /// The provenance string the compile path records: the reported
    /// version, or empty for every failure branch (settled design —
    /// artifact identity is pinned by `compiler_blake3`).
    fn into_version(self) -> String {
        match self {
            VersionProbeOutcome::Reported(version) => version,
            _ => String::new(),
        }
    }
}

pub(super) fn probe_compiler_version_outcome(
    roc_bin: &Path,
    timeout: Duration,
) -> VersionProbeOutcome {
    use std::io::Read as _;
    let mut child = match std::process::Command::new(roc_bin)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return VersionProbeOutcome::SpawnFailed,
    };
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return VersionProbeOutcome::SpawnFailed;
        }
    };
    set_nonblocking(&stdout);
    // Drain whatever the child has written so far into the tail buffer.
    // A nonblocking read reports `WouldBlock` when the pipe is empty for
    // now and `Ok(0)` at EOF; both end the drain.
    fn drain(stdout: &mut std::process::ChildStdout, retained: &mut Vec<u8>) {
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    retained.extend_from_slice(&chunk[..read]);
                    if retained.len() > MAX_CAPTURE_BYTES {
                        retained.drain(..retained.len() - MAX_CAPTURE_BYTES);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }
    let mut retained = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    enum ProbeEnd {
        Exited(std::process::ExitStatus),
        TimedOut,
        WaitFailed,
    }
    let end = loop {
        drain(&mut stdout, &mut retained);
        match child.try_wait() {
            Ok(Some(status)) => break ProbeEnd::Exited(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break ProbeEnd::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break ProbeEnd::WaitFailed,
        }
    };
    drain(&mut stdout, &mut retained);
    match end {
        ProbeEnd::Exited(status) if status.success() => {
            VersionProbeOutcome::Reported(String::from_utf8_lossy(&retained).trim().to_owned())
        }
        ProbeEnd::Exited(status) => VersionProbeOutcome::ExitedNonZero(status.code()),
        ProbeEnd::TimedOut => VersionProbeOutcome::TimedOut,
        ProbeEnd::WaitFailed => VersionProbeOutcome::WaitFailed,
    }
}

/// Put a pipe fd into nonblocking mode for the version probe's drain
/// loop. On failure the probe still works: reads simply block until
/// the child writes or exits, and the deadline poll keeps the bound
/// (a blocked read can delay one drain, never the kill decision past
/// the child's own output activity).
#[cfg(unix)]
fn set_nonblocking(stdout: &std::process::ChildStdout) {
    use std::os::fd::AsRawFd as _;
    // Safety: `fcntl` on a live fd owned by `stdout`; only the
    // file-status flags are read and OR-ed, never replaced.
    #[allow(unsafe_code)]
    unsafe {
        let flags = libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// No nonblocking control on non-Unix targets; the probe's deadline
/// poll still bounds the wait between drains.
#[cfg(not(unix))]
fn set_nonblocking(_stdout: &std::process::ChildStdout) {}

/// Read `reader` to EOF, retaining only the last `cap` bytes.
///
/// Draining never stops at the cap: the writer (a compiler child on a
/// pipe) must never block on a full pipe, and the retained memory stays
/// bounded at `cap` bytes plus one read chunk no matter how much the
/// writer emits. Interrupted reads are retried (as `read_to_end` does);
/// any other read error ends the capture with what was retained.
pub(super) fn read_capture_tail(reader: &mut impl std::io::Read, cap: usize) -> Vec<u8> {
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

pub(super) fn tail(bytes: &[u8], max: usize) -> String {
    let bytes = if bytes.len() > max {
        &bytes[bytes.len() - max..]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).into_owned()
}
