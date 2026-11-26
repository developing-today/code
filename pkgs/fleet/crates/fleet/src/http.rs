//! Tier T2 (http/fleet) — developing-today/code#13717.
//!
//! The fleet-shape client for the laptop-agent daemon's HTTP API.
//! The daemon is the endpoint; this module is its contract surface
//! in the fleet binary: token auth (read from the token file at call
//! time, never in argv), the primary-checkout guard applied before
//! anything is sent, a timeout on every call (the daemon's own
//! `timeout_s` plus the transport grace as the local deadline), and
//! exit codes propagated from the daemon's reply.
//!
//! The ladder is built from the daemon client's HTTP legs only
//! (broker socket → Cloudflare tunnel → Tailscale CONNECT) via
//! `legs::http_ladder`: the SSH leg is excluded by construction, so
//! this tier can never silently become the ssh tier.

use std::io::{IsTerminal, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fleet_config::Config;
use fleet_laptop_agent::ladder::Ladder;
use fleet_laptop_agent::{client, guard, legs, new_trip};
use serde_json::{Value, json};

use crate::forms::{ServeOutcome, T2, announce};

const EXIT_TIMEOUT: i32 = 124;
const EXIT_USAGE: i32 = 2;
const DEFAULT_TIMEOUT_S: u64 = 300;

const USAGE: &str = "\
Usage: fleet http <command> [args...]

Tier T2 (http/fleet) of the laptop route forms (#13717): the
laptop-agent daemon's HTTP API as a fleet subcommand. The ladder is
HTTP legs only — this tier never falls through to ssh.

Commands:
  health   Daemon reachability (GET /health through the ladder)
  jobs     List daemon jobs
  kill JOB Terminate a daemon job
  exec [--cwd D] [--timeout S] [--shell] -- cmd...
           Run synchronously; the daemon's exit code propagates.
           --timeout is in seconds and always applies (default
           300); a daemon-side timeout exits 124. The daemon has no
           stdin channel: exec refuses piped or redirected stdin
           that carries data (exit 2). Stage data with
           `fleet proxy put`, or use `fleet ssh exec` for
           interactive input.
  run [--cwd D] [--shell] -- cmd...
           Submit as a background job; prints the job id
  poll JOB [--wait S]   Poll a job, optionally waiting
  sessions [--json]     Daemon jobs as connect sessions: id, origin,
                        state, start, spooled bytes, bytes dropped,
                        owning context id (#15420)
  attach JOB [--wait S] Stream a session's output from the saved
                        cursor; a finished session is read to the end
  fetch REMOTE LOCAL    Fetch a file written by a job";

fn build_ladder(config: &Config) -> Result<Arc<Ladder>, String> {
    legs::http_ladder(config).map(Arc::new)
}

/// Mirror of the laptop-agent domain's external-deadline clamp: a
/// harness-imposed `LAPTOP_AGENT_DEADLINE_S` bounds every call.
fn clamp_external(timeout_s: u64) -> u64 {
    if let Ok(raw) = std::env::var(fleet_laptop_agent::EXTERNAL_DEADLINE_ENV) {
        if let Ok(cap) = raw.trim().parse::<f64>() {
            let cap = cap.max(1.0) as u64;
            return timeout_s.min(cap).max(1);
        }
    }
    timeout_s.max(1)
}

/// Render a daemon /exec reply the way the laptop-agent domain does
/// (stdout/stderr through, timeout → 124), with this tier's prefix
/// on the side lines.
fn render_exec_reply(body: &Value) -> i32 {
    let Some(result) = body
        .get("results")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    else {
        eprintln!("http: daemon reply carried no result: {body}");
        return 1;
    };
    if let Some(s) = result.get("stdout").and_then(Value::as_str) {
        print!("{s}");
    }
    if let Some(s) = result.get("stderr").and_then(Value::as_str) {
        eprint!("{s}");
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    if result
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        eprintln!("http: warning: command output was truncated by the daemon");
    }
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        if !err.is_empty() {
            eprintln!("http: daemon error: {err}");
            return 1;
        }
    }
    if result
        .get("timeout")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return EXIT_TIMEOUT;
    }
    result
        .get("exit")
        .and_then(Value::as_i64)
        .map(|c| c as i32)
        .unwrap_or(1)
}

fn serve_with_ladder(
    ladder: &Arc<Ladder>,
    cwd: Option<&str>,
    shell: bool,
    timeout_s: u64,
    cmd: &[String],
) -> ServeOutcome {
    if let Err(m) = guard::check_job(cwd, cmd) {
        // A policy refusal is the tier answering, not failing: the
        // guard exists so no route drives the primary checkout, and
        // tier 0 must not route around it.
        eprintln!("http: {m}");
        return ServeOutcome::Served(1);
    }
    let timeout_s = clamp_external(timeout_s);
    let spec = client::command_spec(cwd, shell, cmd);
    let trip = new_trip();
    let deadline = Duration::from_secs(timeout_s + fleet_laptop_agent::TRANSPORT_GRACE_S);
    let body = json!({ "cmds": [spec], "timeout_s": timeout_s });
    match client::call(ladder, &trip, "POST", "/exec", Some(body), false, deadline) {
        Ok(reply) => ServeOutcome::Served(render_exec_reply(&reply.body)),
        Err(client::CallFailure::Deadline) => {
            client::cancel(ladder, &trip);
            ServeOutcome::TimedOut
        }
        Err(e) => ServeOutcome::Failed(e.to_string()),
    }
}

/// Serve one exec through this tier. Used by the standalone verb
/// and by tier 0.
pub fn serve(config: &Config, cwd: Option<&str>, timeout_s: u64, cmd: &[String]) -> ServeOutcome {
    match build_ladder(config) {
        Ok(ladder) => serve_with_ladder(&ladder, cwd, false, timeout_s, cmd),
        Err(m) => ServeOutcome::Failed(m),
    }
}

struct ExecArgs {
    cwd: Option<String>,
    timeout_s: u64,
    shell: bool,
    cmd: Vec<String>,
}

fn parse_exec_args(args: &[String], with_timeout: bool) -> Result<ExecArgs, String> {
    let mut cwd = None;
    let mut timeout_s = DEFAULT_TIMEOUT_S;
    let mut shell = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            let cmd: Vec<String> = args[i + 1..].to_vec();
            if cmd.is_empty() {
                return Err("no command after --".into());
            }
            return Ok(ExecArgs {
                cwd,
                timeout_s,
                shell,
                cmd,
            });
        } else if a == "--cwd" {
            i += 1;
            cwd = Some(args.get(i).ok_or("--cwd needs a value")?.to_string());
        } else if let Some(v) = a.strip_prefix("--cwd=") {
            cwd = Some(v.to_string());
        } else if a == "--timeout" && with_timeout {
            i += 1;
            timeout_s = args
                .get(i)
                .ok_or("--timeout needs a value")?
                .parse()
                .map_err(|_| "--timeout must be seconds".to_string())?;
        } else if let Some(v) = a.strip_prefix("--timeout=") {
            if !with_timeout {
                return Err("--timeout is not taken here".into());
            }
            timeout_s = v
                .parse()
                .map_err(|_| "--timeout must be seconds".to_string())?;
        } else if a == "--shell" {
            shell = true;
        } else {
            return Err(format!("unexpected argument {a} (command goes after --)"));
        }
        i += 1;
    }
    Err("missing -- before the command".into())
}

/// What the caller's stdin holds, for the stdin contract of
/// developing-today/code#10356 §4: the daemon has no stdin channel,
/// so `exec` must refuse data rather than silently drop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdinKind {
    /// A terminal: nothing is piped, and nothing is read from it.
    Terminal,
    /// Closed, `/dev/null`, an empty file, or a pipe with nothing
    /// ready: nothing would be lost.
    Empty,
    /// A file or pipe that carries bytes the daemon would never see.
    Data,
}

/// How long to wait for a pipe to show data before calling it empty.
const STDIN_PROBE_MS: i32 = 50;

/// Classify `fd` without consuming anything from it.
fn classify_stdin_fd(fd: RawFd) -> StdinKind {
    // SAFETY: isatty, fstat, poll and ioctl only inspect the fd.
    if unsafe { libc::isatty(fd) } == 1 {
        return StdinKind::Terminal;
    }
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        // Closed or invalid: there is nothing to forward.
        return StdinKind::Empty;
    }
    match st.st_mode & libc::S_IFMT {
        libc::S_IFREG => {
            // Bytes left from the current offset onward.
            let pos = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) };
            let pos = if pos < 0 { 0 } else { pos };
            if st.st_size > pos {
                StdinKind::Data
            } else {
                StdinKind::Empty
            }
        }
        libc::S_IFIFO | libc::S_IFSOCK => {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, STDIN_PROBE_MS) };
            if ready <= 0 || pfd.revents & libc::POLLIN == 0 {
                // Nothing within the probe window, or hung up empty.
                return StdinKind::Empty;
            }
            let mut avail: libc::c_int = 0;
            if unsafe { libc::ioctl(fd, libc::FIONREAD, &mut avail) } == 0 && avail > 0 {
                StdinKind::Data
            } else {
                StdinKind::Empty
            }
        }
        // Character devices (/dev/null) and anything else.
        _ => StdinKind::Empty,
    }
}

fn classify_stdin() -> StdinKind {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return StdinKind::Terminal;
    }
    classify_stdin_fd(stdin.as_raw_fd())
}

/// The usage error `exec` prints when stdin carries data
/// (developing-today/code#10356 §4).
const STDIN_REFUSAL: &str = "\
http: exec: stdin carries data, but the laptop-agent daemon has no stdin
channel, so it would be dropped (developing-today/code#10356 section 4).
Stage the data first, then read it from the laptop path:
  fleet proxy put <local-file|-> <laptop-path>
  fleet http exec -- sh -c 'cmd < <laptop-path>'
For interactive input, use `fleet ssh exec` (tier T3).";

/// `None` lets exec go ahead; `Some(message)` is the refusal.
fn stdin_refusal(kind: StdinKind) -> Option<&'static str> {
    match kind {
        StdinKind::Data => Some(STDIN_REFUSAL),
        StdinKind::Terminal | StdinKind::Empty => None,
    }
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    let verb = args.first().map(|s| s.as_str());
    if verb.is_none() || matches!(verb, Some("--help") | Some("-h")) {
        println!("{USAGE}");
        return if verb.is_none() { EXIT_USAGE } else { 0 };
    }
    if verb == Some("exec") {
        // Usage errors (bad flags, data on stdin) win before the
        // ladder is built, so nothing is sent and nothing is dialed.
        if let Err(m) = parse_exec_args(&args[1..], true) {
            eprintln!("http: exec: {m}\n{USAGE}");
            return EXIT_USAGE;
        }
        if let Some(m) = stdin_refusal(classify_stdin()) {
            eprintln!("{m}");
            return EXIT_USAGE;
        }
    }
    let ladder = match build_ladder(config) {
        Ok(l) => l,
        Err(m) => {
            eprintln!("http: {m}");
            return 1;
        }
    };
    match verb.unwrap() {
        "health" => client::health(&ladder),
        "jobs" => client::jobs(&ladder),
        "kill" => match args.get(1) {
            Some(job) => client::kill(&ladder, job),
            None => {
                eprintln!("http: kill needs a job id\n{USAGE}");
                EXIT_USAGE
            }
        },
        "exec" => {
            let parsed = match parse_exec_args(&args[1..], true) {
                Ok(p) => p,
                Err(m) => {
                    eprintln!("http: exec: {m}\n{USAGE}");
                    return EXIT_USAGE;
                }
            };
            announce(T2);
            match serve_with_ladder(
                &ladder,
                parsed.cwd.as_deref(),
                parsed.shell,
                parsed.timeout_s,
                &parsed.cmd,
            ) {
                ServeOutcome::Served(code) => code,
                ServeOutcome::TimedOut => {
                    eprintln!(
                        "http: timed out after {}s (daemon job cancelled)",
                        parsed.timeout_s
                    );
                    EXIT_TIMEOUT
                }
                ServeOutcome::Failed(m) => {
                    eprintln!("http: {m}");
                    1
                }
            }
        }
        "run" => {
            let parsed = match parse_exec_args(&args[1..], false) {
                Ok(p) => p,
                Err(m) => {
                    eprintln!("http: run: {m}\n{USAGE}");
                    return EXIT_USAGE;
                }
            };
            if let Err(m) = guard::check_job(parsed.cwd.as_deref(), &parsed.cmd) {
                eprintln!("http: {m}");
                return 1;
            }
            announce(T2);
            client::run(
                &ladder,
                client::command_spec(parsed.cwd.as_deref(), parsed.shell, &parsed.cmd),
            )
        }
        "sessions" => match args.get(1).map(String::as_str) {
            None => client::sessions(&ladder, false),
            Some("--json") => client::sessions(&ladder, true),
            Some(other) => {
                eprintln!("http: sessions: unexpected argument {other}\n{USAGE}");
                EXIT_USAGE
            }
        },
        "poll" | "attach" => {
            let Some(job) = args.get(1) else {
                eprintln!("http: {} needs a job id\n{USAGE}", args[0]);
                return EXIT_USAGE;
            };
            let mut wait: u64 = 30;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--wait" {
                    i += 1;
                    wait = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(wait);
                } else if let Some(v) = args[i].strip_prefix("--wait=") {
                    wait = v.parse().unwrap_or(wait);
                }
                i += 1;
            }
            client::poll(&ladder, job, wait)
        }
        "fetch" => match (args.get(1), args.get(2)) {
            (Some(remote), Some(local)) => client::fetch(&ladder, remote, &PathBuf::from(local)),
            _ => {
                eprintln!("http: fetch needs REMOTE and LOCAL paths\n{USAGE}");
                EXIT_USAGE
            }
        },
        other => {
            eprintln!("http: unknown command {other}\n{USAGE}");
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{FromRawFd, OwnedFd};

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: pipe() just returned two fresh fds we now own.
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fleet-http-stdin-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn http_exec_refuses_piped_stdin_naming_proxy_put() {
        let m = stdin_refusal(StdinKind::Data).expect("data on stdin is refused");
        assert!(m.contains("fleet proxy put"), "{m}");
        assert!(m.contains("fleet ssh exec"), "{m}");
        assert!(m.contains("10356"), "{m}");
        assert_eq!(stdin_refusal(StdinKind::Terminal), None);
        assert_eq!(stdin_refusal(StdinKind::Empty), None);
    }

    #[test]
    fn usage_names_the_stdin_rule() {
        assert!(USAGE.contains("stdin channel"));
        assert!(USAGE.contains("fleet proxy put"));
    }

    #[test]
    fn pipe_with_data_is_data() {
        let (r, w) = pipe();
        let n = unsafe { libc::write(w.as_raw_fd(), b"hello".as_ptr().cast(), 5) };
        assert_eq!(n, 5);
        assert_eq!(classify_stdin_fd(r.as_raw_fd()), StdinKind::Data);
        // Classifying consumed nothing.
        let mut avail: libc::c_int = 0;
        assert_eq!(
            unsafe { libc::ioctl(r.as_raw_fd(), libc::FIONREAD, &mut avail) },
            0
        );
        assert_eq!(avail, 5);
    }

    #[test]
    fn pipe_with_data_and_closed_writer_is_data() {
        let (r, w) = pipe();
        let n = unsafe { libc::write(w.as_raw_fd(), b"x".as_ptr().cast(), 1) };
        assert_eq!(n, 1);
        drop(w);
        assert_eq!(classify_stdin_fd(r.as_raw_fd()), StdinKind::Data);
    }

    #[test]
    fn empty_pipe_with_closed_writer_is_empty() {
        let (r, w) = pipe();
        drop(w);
        assert_eq!(classify_stdin_fd(r.as_raw_fd()), StdinKind::Empty);
    }

    #[test]
    fn empty_pipe_with_open_writer_is_empty() {
        let (r, _w) = pipe();
        assert_eq!(classify_stdin_fd(r.as_raw_fd()), StdinKind::Empty);
    }

    #[test]
    fn regular_files_by_content() {
        let full = temp_path("full");
        std::fs::write(&full, b"data").unwrap();
        let f = std::fs::File::open(&full).unwrap();
        assert_eq!(classify_stdin_fd(f.as_raw_fd()), StdinKind::Data);
        drop(f);
        let _ = std::fs::remove_file(&full);

        let empty = temp_path("empty");
        std::fs::write(&empty, b"").unwrap();
        let f = std::fs::File::open(&empty).unwrap();
        assert_eq!(classify_stdin_fd(f.as_raw_fd()), StdinKind::Empty);
        drop(f);
        let _ = std::fs::remove_file(&empty);
    }

    #[test]
    fn dev_null_is_empty() {
        let f = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(classify_stdin_fd(f.as_raw_fd()), StdinKind::Empty);
    }
}
