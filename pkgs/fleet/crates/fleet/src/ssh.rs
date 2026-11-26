//! Tier T3 (ssh/fleet) — developing-today/code#13717.
//!
//! SSH's endpoint-grade shape is a **binary-owned client contract**:
//! the binary constructs the ssh invocation itself — pinned host
//! key, proxy CONNECT helper, identities-only key, connect timeout
//! — instead of delegating to a shell script, and enforces a
//! mandatory overall deadline (child kill → exit 124). The record
//! stays honest about the one thing SSH cannot do: sshd owns the
//! remote process, so a client-side kill ends the session but
//! cannot cancel the remote command protocol-side.
//!
//! Session hygiene: connections multiplex over a fleet-owned
//! ControlMaster socket (`~/.ssh/cm-fleet-…`, persist 600 s — the
//! fleet path is separate from laptop-ssh's, so the two surfaces
//! never share a master), and `fleet ssh close` tears the master
//! down.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use fleet_config::Config;

use crate::forms::{ServeOutcome, T3, announce, shell_quote, spawn_with_deadline};

const TARGET: &str = "user@100.99.113.103";
const EXIT_TIMEOUT: i32 = 124;
const EXIT_USAGE: i32 = 2;
const DEFAULT_TIMEOUT_S: u64 = 300;

const USAGE: &str = "\
Usage: fleet ssh <command> [args...]

Tier T3 (ssh/fleet) of the laptop route forms (#13717): real SSH to
the laptop under a binary-owned client contract (pinned host key,
proxy CONNECT, multiplexing on a fleet-owned socket).

Commands:
  health   Run `hostname` over ssh; exit 0 iff the host answers amd
  sessions List Fleet-owned tmux sessions (`fleet-*`), read-only
  sweep --idle-for DURATION [--dry-run]
           Remove detached, idle `fleet-*` sessions only; DURATION
           accepts seconds (s), minutes (m), hours (h), days (d),
           or bare seconds. The default is a real sweep.
  exec [--cwd D] [--timeout S] -- cmd...
           Run a command line over ssh (the remote shell parses
           it); the remote exit code propagates. --timeout is in
           seconds and always applies (default 300): on expiry
           the ssh client is killed and the exit code is 124.
           Note: ssh's own failure exit is 255.
  close    Tear down the fleet-owned multiplex master, if any";

fn ssh_base(home: &PathBuf, connect_timeout_s: u64) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o")
        .arg(format!(
            "ProxyCommand=python3 {}",
            home.join("tools/laptop-proxy-connect.py").display()
        ))
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg(format!(
            "UserKnownHostsFile={}",
            home.join(".ssh/known_hosts_laptop").display()
        ))
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-i")
        .arg(home.join(".ssh/id_ed25519"))
        .arg("-o")
        .arg(format!("ConnectTimeout={}", connect_timeout_s.clamp(1, 60)))
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=8")
        .arg("-o")
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg(format!(
            "ControlPath={}",
            home.join(".ssh/cm-fleet-%r@%h:%p").display()
        ))
        .arg("-o")
        .arg("ControlPersist=600")
        .arg(TARGET);
    cmd
}

/// Serve one exec through this tier (stdio inherited: ssh output is
/// the command's output). Used by the standalone verb and tier 0.
pub fn serve(_config: &Config, cwd: Option<&str>, timeout_s: u64, cmd: &[String]) -> ServeOutcome {
    if std::env::var_os("HTTPS_PROXY").is_none() {
        return ServeOutcome::Failed(
            "HTTPS_PROXY is not set (the proxy CONNECT helper reads it)".into(),
        );
    }
    let home = fleet_config::Config::home_dir();
    let line = remote_line(&fleet_config::ctx::Ctx::from_env(), cwd, cmd);
    let mut ssh = ssh_base(&home, timeout_s);
    ssh.arg(line);
    spawn_with_deadline(&mut ssh, timeout_s)
}

/// The remote command line (the remote shell parses it). The
/// caller's `FLEET_CTX_*` record leads as an explicit `export …;`
/// prefix (code#14188): the SSH carrier that works whatever the
/// laptop sshd's `AcceptEnv` allows, and covers the `cd` and the
/// whole command. No context, no prefix — the line is unchanged.
fn remote_line(ctx: &fleet_config::ctx::Ctx, cwd: Option<&str>, cmd: &[String]) -> String {
    let mut line = cmd.join(" ");
    if let Some(d) = cwd {
        line = format!("cd {} && {}", shell_quote(d), line);
    }
    format!("{}{line}", ctx.shell_prefix())
}

fn health(config: &Config) -> i32 {
    match serve(config, None, 20, &["hostname".to_string()]) {
        ServeOutcome::Served(0) => {
            println!("ssh: ok (form {})", T3.label());
            0
        }
        ServeOutcome::Served(code) => {
            eprintln!("ssh: health: ssh exited {code}");
            code
        }
        ServeOutcome::TimedOut => {
            eprintln!("ssh: health: timed out");
            EXIT_TIMEOUT
        }
        ServeOutcome::Failed(m) => {
            eprintln!("ssh: health: {m}");
            1
        }
    }
}

fn close() -> i32 {
    let home = fleet_config::Config::home_dir();
    let mut cmd = Command::new("ssh");
    cmd.arg("-O")
        .arg("exit")
        .arg("-o")
        .arg(format!(
            "ControlPath={}",
            home.join(".ssh/cm-fleet-%r@%h:%p").display()
        ))
        .arg(TARGET);
    match spawn_with_deadline(&mut cmd, 15) {
        ServeOutcome::Served(code) => code,
        ServeOutcome::TimedOut => EXIT_TIMEOUT,
        ServeOutcome::Failed(m) => {
            eprintln!("ssh: close: {m}");
            1
        }
    }
}

enum CaptureOutcome {
    Complete { code: i32, stdout: String },
    TimedOut,
    Failed(String),
}

/// Capture a small remote result while keeping the same pinned SSH
/// contract and overall deadline as `exec`. A reader thread drains
/// stdout while ssh runs so a long session list cannot fill its pipe.
fn capture(_config: &Config, timeout_s: u64, cmd: &[String]) -> CaptureOutcome {
    if std::env::var_os("HTTPS_PROXY").is_none() {
        return CaptureOutcome::Failed(
            "HTTPS_PROXY is not set (the proxy CONNECT helper reads it)".into(),
        );
    }
    let home = fleet_config::Config::home_dir();
    let line = remote_line(&fleet_config::ctx::Ctx::from_env(), None, cmd);
    let mut ssh = ssh_base(&home, timeout_s);
    ssh.arg(line).stdout(Stdio::piped());
    let mut child = match ssh.spawn() {
        Ok(child) => child,
        Err(e) => return CaptureOutcome::Failed(format!("spawn: {e}")),
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return CaptureOutcome::Failed("ssh stdout pipe was not created".into());
    };
    let reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|()| bytes)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_s.max(1));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return CaptureOutcome::TimedOut;
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return CaptureOutcome::Failed(format!("wait: {e}"));
            }
        }
    };
    let Some(status) = status else {
        return CaptureOutcome::Failed("ssh exited without a status".into());
    };
    let bytes = match reader.join() {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => return CaptureOutcome::Failed(format!("read ssh stdout: {e}")),
        Err(_) => return CaptureOutcome::Failed("ssh stdout reader panicked".into()),
    };
    CaptureOutcome::Complete {
        code: status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn sessions(config: &Config) -> i32 {
    announce(T3);
    let cmd = [
        "tmux".to_string(),
        "list-sessions".to_string(),
        "-F".to_string(),
        "'#{session_name}'".to_string(),
    ];
    match capture(config, DEFAULT_TIMEOUT_S, &cmd) {
        CaptureOutcome::Complete { code: 0, stdout } => {
            for name in stdout
                .lines()
                .map(str::trim)
                .filter(|name| name.starts_with("fleet-"))
            {
                println!("{name}");
            }
            0
        }
        CaptureOutcome::Complete { code, .. } => {
            eprintln!("ssh: sessions: tmux list-sessions exited {code}");
            code
        }
        CaptureOutcome::TimedOut => {
            eprintln!("ssh: sessions: timed out after {DEFAULT_TIMEOUT_S}s");
            EXIT_TIMEOUT
        }
        CaptureOutcome::Failed(m) => {
            eprintln!("ssh: sessions: {m}");
            1
        }
    }
}

struct SweepArgs {
    idle_for_s: u64,
    dry_run: bool,
}

fn parse_duration(value: &str) -> Result<u64, String> {
    let (digits, scale) = if let Some(digits) = value.strip_suffix('s') {
        (digits, 1_u64)
    } else if let Some(digits) = value.strip_suffix('m') {
        (digits, 60)
    } else if let Some(digits) = value.strip_suffix('h') {
        (digits, 60 * 60)
    } else if let Some(digits) = value.strip_suffix('d') {
        (digits, 24 * 60 * 60)
    } else {
        (value, 1)
    };
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "sweep: --idle-for must be a positive duration".to_string())?;
    let seconds = amount
        .checked_mul(scale)
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| "sweep: --idle-for must be a positive duration".to_string())?;
    Ok(seconds)
}

fn parse_sweep_args(args: &[String]) -> Result<SweepArgs, String> {
    let mut idle_for_s = None;
    let mut dry_run = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--idle-for" => {
                i += 1;
                let value = args.get(i).ok_or("sweep: --idle-for needs a value")?;
                if idle_for_s.is_some() {
                    return Err("sweep: --idle-for may be supplied once".into());
                }
                idle_for_s = Some(parse_duration(value)?);
            }
            value if value.starts_with("--idle-for=") => {
                if idle_for_s.is_some() {
                    return Err("sweep: --idle-for may be supplied once".into());
                }
                idle_for_s = Some(parse_duration(&value[11..])?);
            }
            "--dry-run" => {
                if dry_run {
                    return Err("sweep: --dry-run may be supplied once".into());
                }
                dry_run = true;
            }
            other => return Err(format!("sweep: unexpected argument {other}")),
        }
        i += 1;
    }
    let idle_for_s = idle_for_s.ok_or("sweep: --idle-for is required")?;
    Ok(SweepArgs {
        idle_for_s,
        dry_run,
    })
}

struct TmuxSession {
    name: String,
    activity: u64,
    attached: u32,
}

fn parse_session_activity(output: &str) -> Result<Vec<TmuxSession>, String> {
    let mut sessions = Vec::new();
    for (index, line) in output.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or_default();
        let activity = fields
            .next()
            .ok_or_else(|| format!("sweep: malformed tmux session row {}", index + 1))?
            .parse::<u64>()
            .map_err(|_| format!("sweep: invalid activity timestamp on row {}", index + 1))?;
        let attached = fields
            .next()
            .ok_or_else(|| format!("sweep: malformed tmux session row {}", index + 1))?
            .parse::<u32>()
            .map_err(|_| format!("sweep: invalid attached-client count on row {}", index + 1))?;
        if fields.next().is_some() || name.is_empty() {
            return Err(format!("sweep: malformed tmux session row {}", index + 1));
        }
        sessions.push(TmuxSession {
            name: name.to_string(),
            activity,
            attached,
        });
    }
    Ok(sessions)
}

fn sweep_candidate(session: &TmuxSession, now: u64, idle_for_s: u64) -> bool {
    session.name.starts_with("fleet-")
        && session.attached == 0
        && now.saturating_sub(session.activity) >= idle_for_s
}

fn list_session_activity(config: &Config) -> CaptureOutcome {
    let cmd = [
        "tmux".to_string(),
        "list-sessions".to_string(),
        "-F".to_string(),
        "'#{session_name}\\t#{session_activity}\\t#{session_attached}'".to_string(),
    ];
    capture(config, 30, &cmd)
}

fn check_and_kill(config: &Config, name: &str) -> CaptureOutcome {
    let target = crate::forms::shell_quote(&format!("={name}"));
    let script = format!(
        "if [ \"$(tmux display-message -p -t {target} '#{{session_attached}}')\" = 0 ]; then tmux kill-session -t {target}; else exit 42; fi"
    );
    let cmd = [
        "sh".to_string(),
        "-c".to_string(),
        crate::forms::shell_quote(&script),
    ];
    capture(config, 30, &cmd)
}

fn sweep(config: &Config, args: &SweepArgs) -> i32 {
    announce(T3);
    let sessions = match list_session_activity(config) {
        CaptureOutcome::Complete { code: 0, stdout } => match parse_session_activity(&stdout) {
            Ok(sessions) => sessions,
            Err(m) => {
                eprintln!("{m}");
                return 1;
            }
        },
        CaptureOutcome::Complete { code, .. } => {
            eprintln!("ssh: sweep: tmux list-sessions exited {code}");
            return code;
        }
        CaptureOutcome::TimedOut => {
            eprintln!("ssh: sweep: tmux list-sessions timed out");
            return EXIT_TIMEOUT;
        }
        CaptureOutcome::Failed(m) => {
            eprintln!("ssh: sweep: {m}");
            return 1;
        }
    };
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(e) => {
            eprintln!("ssh: sweep: system clock is before the Unix epoch: {e}");
            return 1;
        }
    };
    for session in sessions
        .into_iter()
        .filter(|session| sweep_candidate(session, now, args.idle_for_s))
    {
        if args.dry_run {
            println!("would sweep {}", session.name);
            continue;
        }
        match check_and_kill(config, &session.name) {
            CaptureOutcome::Complete { code: 0, .. } => println!("swept {}", session.name),
            CaptureOutcome::Complete { code: 42, .. } => {
                println!("skipped {} (attached)", session.name)
            }
            CaptureOutcome::Complete { code, .. } => {
                eprintln!(
                    "ssh: sweep: recheck/kill for {} exited {code}",
                    session.name
                );
                return code;
            }
            CaptureOutcome::TimedOut => {
                eprintln!("ssh: sweep: recheck/kill for {} timed out", session.name);
                return EXIT_TIMEOUT;
            }
            CaptureOutcome::Failed(m) => {
                eprintln!("ssh: sweep: {m}");
                return 1;
            }
        }
    }
    0
}

struct ExecArgs {
    cwd: Option<String>,
    timeout_s: u64,
    cmd: Vec<String>,
}

fn parse_exec_args(args: &[String]) -> Result<ExecArgs, String> {
    let mut cwd = None;
    let mut timeout_s = DEFAULT_TIMEOUT_S;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            let cmd: Vec<String> = args[i + 1..].to_vec();
            if cmd.is_empty() {
                return Err("exec: no command after --".into());
            }
            return Ok(ExecArgs {
                cwd,
                timeout_s,
                cmd,
            });
        } else if a == "--cwd" {
            i += 1;
            cwd = Some(args.get(i).ok_or("exec: --cwd needs a value")?.to_string());
        } else if let Some(v) = a.strip_prefix("--cwd=") {
            cwd = Some(v.to_string());
        } else if a == "--timeout" {
            i += 1;
            timeout_s = args
                .get(i)
                .ok_or("exec: --timeout needs a value")?
                .parse()
                .map_err(|_| "exec: --timeout must be seconds".to_string())?;
        } else if let Some(v) = a.strip_prefix("--timeout=") {
            timeout_s = v
                .parse()
                .map_err(|_| "exec: --timeout must be seconds".to_string())?;
        } else {
            return Err(format!(
                "exec: unexpected argument {a} (command goes after --)"
            ));
        }
        i += 1;
    }
    Err("exec: missing -- before the command".into())
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            0
        }
        Some("health") => health(config),
        Some("sessions") if args.len() == 1 => sessions(config),
        Some("sweep") => {
            let parsed = match parse_sweep_args(&args[1..]) {
                Ok(parsed) => parsed,
                Err(m) => {
                    eprintln!("ssh: {m}\n{USAGE}");
                    return EXIT_USAGE;
                }
            };
            sweep(config, &parsed)
        }
        Some("close") => close(),
        Some("exec") => {
            let parsed = match parse_exec_args(&args[1..]) {
                Ok(p) => p,
                Err(m) => {
                    eprintln!("ssh: {m}\n{USAGE}");
                    return EXIT_USAGE;
                }
            };
            announce(T3);
            match serve(config, parsed.cwd.as_deref(), parsed.timeout_s, &parsed.cmd) {
                ServeOutcome::Served(code) => code,
                ServeOutcome::TimedOut => {
                    eprintln!("ssh: timed out after {}s", parsed.timeout_s);
                    EXIT_TIMEOUT
                }
                ServeOutcome::Failed(m) => {
                    eprintln!("ssh: {m}");
                    1
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::remote_line;
    use fleet_config::ctx::Ctx;
    use proptest::prelude::*;

    #[test]
    fn remote_line_without_context_is_unchanged_14188() {
        let cmd = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(remote_line(&Ctx::default(), None, &cmd), "echo hi");
    }

    #[test]
    fn remote_line_exports_context_ahead_of_cd_and_command_14188() {
        let ctx = Ctx::from_pairs([("FLEET_CTX_ID", "task-22655"), ("FLEET_CTX_PID", "42")]);
        let cmd = vec![
            "printf".to_string(),
            "'%s'".to_string(),
            "\"$FLEET_CTX_ID\"".to_string(),
        ];
        let line = remote_line(&ctx, Some("/"), &cmd);
        assert!(
            line.starts_with("export FLEET_CTX_ID='task-22655' FLEET_CTX_PID='42'; cd "),
            "{line}"
        );
        // What the remote login shell does with it: a scrubbed env,
        // the context arrives anyway.
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&line)
            .env_clear()
            .output()
            .expect("sh runs");
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "task-22655");
    }

    proptest! {
        /// A sweep candidate must always be Fleet-owned, detached, and
        /// older than the cutoff measured by the caller's clock.
        #[test]
        fn sweep_never_selects_non_fleet_attached_or_fresh_sessions(
            name in prop_oneof![
                Just("fleet-owned".to_string()),
                Just("fleet-other".to_string()),
                Just("work-owned".to_string()),
            ],
            activity in any::<u64>(),
            attached in any::<u32>(),
            now in any::<u64>(),
            idle_for_s in 1_u64..86401,
        ) {
            let session = super::TmuxSession { name, activity, attached };
            if super::sweep_candidate(&session, now, idle_for_s) {
                prop_assert!(session.name.starts_with("fleet-"));
                prop_assert_eq!(session.attached, 0);
                prop_assert!(now.saturating_sub(session.activity) >= idle_for_s);
            }
        }
    }
}
