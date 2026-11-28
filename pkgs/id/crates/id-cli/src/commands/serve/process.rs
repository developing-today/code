//! Process-liveness probing for the serve lock (#3040, #2384):
//! whether a recorded PID still names the same live process.
//! Split out of `serve.rs` for the F11.10 production-line guard
//! (#10607); behaviour is unchanged.

/// Checks if a process with the given PID is still running.
///
/// Uses platform-specific methods:
/// - Unix: `kill(pid, 0)` which checks existence without sending a signal
/// - Other: Always returns `true` (conservative fallback)
///
/// # Arguments
///
/// * `pid` - The process ID to check
///
/// # Returns
///
/// `true` if the process exists, `false` otherwise.
#[allow(clippy::cast_possible_wrap)] // The Unix branch validates the PID before casting.
#[allow(unsafe_code)] // Required for libc::kill
/// Start time of a process on Linux (/proc/<pid>/stat field 22:
/// clock ticks since boot at which the process started). Together
/// with the PID this identifies one process incarnation: a reused
/// PID has a different start time. Returns None where the value
/// cannot be read (non-Linux, or the process is already gone).
#[cfg(target_os = "linux")]
pub(super) fn get_process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat_start_time(&stat)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn get_process_start_time(_pid: u32) -> Option<u64> {
    None
}

/// Parse the start time (field 22) out of a `/proc/<pid>/stat` string.
///
/// comm (field 2) is parenthesised and may contain spaces or parens;
/// the remaining fields follow the final ')'. starttime is field 22
/// overall, i.e. index 19 of the fields after comm. Extracted as a
/// pure helper so tests can drive it with generated stat strings and
/// an independently chosen field-22 value (#8816): the oracle for
/// this parser must not be the parser itself.
#[cfg(any(target_os = "linux", test))]
#[allow(dead_code)] // On non-Linux test builds the driving tests are Linux-gated.
pub(super) fn parse_proc_stat_start_time(stat: &str) -> Option<u64> {
    let after_comm = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    fields.get(19).and_then(|f| f.parse().ok())
}

pub fn is_process_alive(pid: u32, expected_start_time: Option<u64>) -> bool {
    // #3040: a recorded start time that does not match the process now
    // holding the PID means the PID was reused by an unrelated process;
    // the lock is stale no matter what the existence probe says. When
    // either side has no start time (old lock, non-Linux), fall back
    // to existence only.
    if let Some(expected) = expected_start_time {
        if let Some(actual) = get_process_start_time(pid) {
            if actual != expected {
                return false;
            }
        }
    }
    #[cfg(unix)]
    {
        // PIDs read from the serve lock are untrusted u32 values. Zero checks
        // the caller's process group, while values above i32::MAX wrap to
        // negative process selectors, so neither can identify one process.
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }

        // SAFETY: libc::kill with signal 0 only checks process existence without
        // sending any signal. The check above ensures pid is positive and fits
        // in i32 before the cast.
        // #2384: a failed probe is not proof of death — EPERM means the
        // process exists but is owned by another user, so it is alive;
        // only ESRCH (no such process) and other errnos mean dead. This
        // mirrors mcpx's ProcessAlive (pkgs/mcpx/internal/daemon/paths.go).
        let rc = unsafe { libc::kill(pid as i32, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, just assume it's alive if we have a PID
        let _ = pid;
        true
    }
}
