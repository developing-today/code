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
//!
//! Remote sessions (#15422, B4 of #10356): tmux on the laptop is the
//! attach/detach answer for the SSH path. Fleet-started tmux sessions
//! are named `fleet-<context-id or trip-id>`, and only sessions whose
//! name starts with `fleet-` are Fleet's to list or sweep: `fleet ssh
//! sessions` lists them, read-only, and `fleet ssh sweep --idle-for D`
//! kills the ones with no attached client that have been idle past
//! the bound. Nothing else is touched. A caller that starts a tmux
//! session for fleet work names it `fleet-…`; a caller that starts an
//! unnamed session owns its cleanup.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
  exec [--cwd D] [--timeout S] -- cmd...
           Run a command line over ssh (the remote shell parses
           it); the remote exit code propagates. --timeout is in
           seconds and always applies (default 300): on expiry
           the ssh client is killed and the exit code is 124.
           Note: ssh's own failure exit is 255.
  sessions [--timeout S]
           List the laptop's fleet- tmux sessions (name, attached
           clients, seconds idle), read-only. Only sessions whose
           name starts with fleet- are listed.
  sweep --idle-for D [--dry-run] [--timeout S]
           Kill fleet- tmux sessions with no attached client that
           have been idle longer than D (90s, 30m, 2h, 1d or plain
           seconds). Other sessions are never touched, and a session
           a client attached to since the listing is kept.
  close    Tear down the fleet-owned multiplex master, if any

Fleet-started tmux sessions are named fleet-<context-id or trip-id>.
A caller that starts an unnamed session owns its cleanup.";

/// The prefix that makes a tmux session Fleet's to list or sweep.
const FLEET_SESSION_PREFIX: &str = "fleet-";

/// The tmux listing format: attached clients, last activity (Unix
/// seconds), then the name, last because a name may hold spaces.
const LIST_FORMAT: &str = "#{session_attached} #{session_activity} #{session_name}";

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

/// One tmux session as the remote listing reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TmuxSession {
    name: String,
    attached: u32,
    activity: u64,
}

/// The remote line that prints the laptop's clock (`now <secs>`) and
/// then every tmux session in [`LIST_FORMAT`]. No tmux server means no
/// sessions, not a failure. Idle time is measured against the laptop's
/// own clock, so clock skew between the two hosts never sweeps early.
fn sessions_line() -> String {
    format!(
        "printf 'now %s\\n' \"$(date +%s)\"; tmux list-sessions -F {} 2>/dev/null || true",
        shell_quote(LIST_FORMAT)
    )
}

/// Parses the output of [`sessions_line`]: the laptop's clock and
/// every session listed. A listing that does not read is an error, so
/// nothing is ever swept from a misread listing.
fn parse_listing(out: &str) -> Result<(u64, Vec<TmuxSession>), String> {
    let mut lines = out.lines();
    let now = lines
        .next()
        .and_then(|line| line.strip_prefix("now "))
        .and_then(|v| v.trim().parse().ok())
        .ok_or("the listing does not start with the laptop's clock")?;
    let mut sessions = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let mut parts = line.splitn(3, ' ');
        let (Some(attached), Some(activity), Some(name)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("unreadable tmux listing line: {line:?}"));
        };
        sessions.push(TmuxSession {
            name: name.to_string(),
            attached: attached
                .parse()
                .map_err(|_| format!("unreadable attached count in: {line:?}"))?,
            activity: activity
                .parse()
                .map_err(|_| format!("unreadable activity time in: {line:?}"))?,
        });
    }
    Ok((now, sessions))
}

/// The sessions that are Fleet's: those named `fleet-…`.
fn fleet_sessions(sessions: &[TmuxSession]) -> Vec<&TmuxSession> {
    sessions
        .iter()
        .filter(|s| s.name.starts_with(FLEET_SESSION_PREFIX))
        .collect()
}

/// The sessions a sweep kills: Fleet's, with no attached client, idle
/// longer than `idle_for_s` by the laptop's clock `now`.
fn sweep_targets(now: u64, sessions: &[TmuxSession], idle_for_s: u64) -> Vec<&TmuxSession> {
    fleet_sessions(sessions)
        .into_iter()
        .filter(|s| s.attached == 0 && now.saturating_sub(s.activity) > idle_for_s)
        .collect()
}

/// The remote line that kills `names`. Each kill re-checks, at kill
/// time, that the session still has no attached client, and targets
/// the exact name (`=name`), so a prefix match never reaches another
/// session. A name without the `fleet-` prefix is never included.
fn sweep_line(names: &[&str]) -> String {
    names
        .iter()
        .filter(|name| name.starts_with(FLEET_SESSION_PREFIX))
        .map(|name| {
            let target = shell_quote(&format!("={name}"));
            format!(
                "if [ \"$(tmux display-message -p -t {target} '#{{session_attached}}' 2>/dev/null)\" = 0 ]; then tmux kill-session -t {target} && printf 'swept %s\\n' {}; fi",
                shell_quote(name)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Parses an idle bound: plain seconds, or a number with `s`, `m`, `h`
/// or `d`. Zero is refused, so a sweep never means "everything idle".
fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&text[..i], c),
        _ => (text, 's'),
    };
    let value: u64 = digits
        .parse()
        .map_err(|_| format!("--idle-for: {text:?} is not a duration (90s, 30m, 2h, 1d)"))?;
    let scale = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => {
            return Err(format!(
                "--idle-for: unknown unit in {text:?} (s, m, h or d)"
            ));
        }
    };
    match value.checked_mul(scale) {
        Some(0) => Err("--idle-for must be more than zero".into()),
        Some(seconds) => Ok(seconds),
        None => Err(format!("--idle-for: {text:?} is too large")),
    }
}

/// How a captured remote run ended.
enum Captured {
    /// The remote command ran; its exit code and stdout.
    Ran(i32, String),
    TimedOut,
    Failed(String),
}

/// Run one remote line over this tier with stdout captured (stderr
/// inherited), under the same deadline contract as [`serve`].
fn capture(line: &str, timeout_s: u64) -> Captured {
    if std::env::var_os("HTTPS_PROXY").is_none() {
        return Captured::Failed(
            "HTTPS_PROXY is not set (the proxy CONNECT helper reads it)".into(),
        );
    }
    let home = fleet_config::Config::home_dir();
    let line = remote_line(
        &fleet_config::ctx::Ctx::from_env(),
        None,
        &[line.to_string()],
    );
    let mut ssh = ssh_base(&home, timeout_s);
    ssh.arg(line).stdout(Stdio::piped());
    capture_with_deadline(&mut ssh, timeout_s)
}

fn capture_with_deadline(cmd: &mut Command, timeout_s: u64) -> Captured {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Captured::Failed(format!("spawn: {e}")),
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Captured::Failed("no stdout pipe".into());
    };
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_s.max(1));
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Captured::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Captured::Failed(format!("wait: {e}")),
        }
    };
    match reader.join() {
        Ok(Ok(out)) => Captured::Ran(code, out),
        Ok(Err(e)) => Captured::Failed(format!("read output: {e}")),
        Err(_) => Captured::Failed("output reader panicked".into()),
    }
}

/// Run a captured remote line and turn anything but a clean run into
/// an exit code, printing why.
fn capture_ok(verb: &str, line: &str, timeout_s: u64) -> Result<String, i32> {
    match capture(line, timeout_s) {
        Captured::Ran(0, out) => Ok(out),
        Captured::Ran(code, _) => {
            eprintln!("ssh: {verb}: ssh exited {code}");
            Err(code)
        }
        Captured::TimedOut => {
            eprintln!("ssh: {verb}: timed out after {timeout_s}s");
            Err(EXIT_TIMEOUT)
        }
        Captured::Failed(m) => {
            eprintln!("ssh: {verb}: {m}");
            Err(1)
        }
    }
}

struct SessionArgs {
    idle_for_s: Option<u64>,
    dry_run: bool,
    timeout_s: u64,
}

fn parse_session_args(verb: &str, args: &[String]) -> Result<SessionArgs, String> {
    let mut parsed = SessionArgs {
        idle_for_s: None,
        dry_run: false,
        timeout_s: 60,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let (flag, inline) = match a.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (a, None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            i += 1;
            args.get(i)
                .cloned()
                .ok_or(format!("{verb}: {name} needs a value"))
        };
        match flag {
            "--idle-for" if verb == "sweep" => {
                parsed.idle_for_s = Some(parse_duration(&value("--idle-for")?)?);
            }
            "--dry-run" if verb == "sweep" && inline.is_none() => parsed.dry_run = true,
            "--timeout" => {
                parsed.timeout_s = value("--timeout")?
                    .parse()
                    .map_err(|_| format!("{verb}: --timeout must be seconds"))?;
            }
            _ => return Err(format!("{verb}: unexpected argument {a}")),
        }
        i += 1;
    }
    if verb == "sweep" && parsed.idle_for_s.is_none() {
        return Err("sweep: --idle-for is required".into());
    }
    Ok(parsed)
}

fn sessions(args: &[String]) -> i32 {
    let parsed = match parse_session_args("sessions", args) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("ssh: {m}\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    announce(T3);
    let out = match capture_ok("sessions", &sessions_line(), parsed.timeout_s) {
        Ok(out) => out,
        Err(code) => return code,
    };
    let (now, listed) = match parse_listing(&out) {
        Ok(listing) => listing,
        Err(m) => {
            eprintln!("ssh: sessions: {m}");
            return 1;
        }
    };
    let fleet = fleet_sessions(&listed);
    if fleet.is_empty() {
        println!("no fleet- tmux sessions");
    }
    for s in fleet {
        println!(
            "{}\tattached={}\tidle={}s",
            s.name,
            s.attached,
            now.saturating_sub(s.activity)
        );
    }
    0
}

fn sweep(args: &[String]) -> i32 {
    let parsed = match parse_session_args("sweep", args) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("ssh: {m}\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let idle_for_s = parsed.idle_for_s.unwrap_or(u64::MAX);
    announce(T3);
    let out = match capture_ok("sweep", &sessions_line(), parsed.timeout_s) {
        Ok(out) => out,
        Err(code) => return code,
    };
    let (now, listed) = match parse_listing(&out) {
        Ok(listing) => listing,
        Err(m) => {
            eprintln!("ssh: sweep: {m}");
            return 1;
        }
    };
    let names: Vec<&str> = sweep_targets(now, &listed, idle_for_s)
        .into_iter()
        .map(|s| s.name.as_str())
        .collect();
    if names.is_empty() {
        println!("nothing to sweep");
        return 0;
    }
    if parsed.dry_run {
        for name in names {
            println!("would sweep {name}");
        }
        return 0;
    }
    match capture_ok("sweep", &sweep_line(&names), parsed.timeout_s) {
        Ok(out) => {
            print!("{out}");
            0
        }
        Err(code) => code,
    }
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
        Some("close") => close(),
        Some("sessions") => sessions(&args[1..]),
        Some("sweep") => sweep(&args[1..]),
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
    use super::{
        TmuxSession, parse_duration, parse_listing, parse_session_args, remote_line, sessions_line,
        sweep_line, sweep_targets,
    };
    use fleet_config::ctx::Ctx;

    fn session(name: &str, attached: u32, activity: u64) -> TmuxSession {
        TmuxSession {
            name: name.to_string(),
            attached,
            activity,
        }
    }

    /// The #15422 done check: given tmux sessions `fleet-a` and `work`,
    /// listing shows only `fleet-a`, and a sweep never touches `work`,
    /// nor a `fleet-*` session with a client attached.
    #[test]
    fn sessions_and_sweep_touch_only_idle_detached_fleet_sessions_15422() {
        let listing =
            "now 10000\n0 1000 fleet-a\n0 1000 work\n1 1000 fleet-attached\n0 9990 fleet-recent\n";
        let (now, listed) = parse_listing(listing).unwrap();
        assert_eq!(now, 10_000);
        let names: Vec<_> = super::fleet_sessions(&listed)
            .into_iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["fleet-a", "fleet-attached", "fleet-recent"]);
        let swept: Vec<_> = sweep_targets(now, &listed, 60)
            .into_iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(swept, ["fleet-a"]);
        // A name that only contains the prefix is not Fleet's.
        let other = [session("my-fleet-a", 0, 0), session("Fleet-b", 0, 0)];
        assert!(sweep_targets(now, &other, 1).is_empty());
    }

    #[test]
    fn a_listing_that_does_not_read_is_refused_15422() {
        assert!(parse_listing("").is_err());
        assert!(parse_listing("0 1 fleet-a\n").is_err());
        assert!(parse_listing("now 5\nx 1 fleet-a\n").is_err());
        assert!(parse_listing("now 5\n0 1\n").is_err());
        let (_, listed) = parse_listing("now 5\n0 1 fleet-a b\n").unwrap();
        assert_eq!(listed, [session("fleet-a b", 0, 1)]);
        let (_, none) = parse_listing("now 5\n").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn idle_bounds_parse_with_units_and_refuse_zero_15422() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("90s"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1_800));
        assert_eq!(parse_duration("2h"), Ok(7_200));
        assert_eq!(parse_duration("1d"), Ok(86_400));
        for bad in ["0", "0m", "", "m", "5w", "-1h", "1.5h"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_session_args("sweep", &args(&["--dry-run"])).is_err());
        let parsed = parse_session_args("sweep", &args(&["--idle-for=2h", "--dry-run"])).unwrap();
        assert_eq!(parsed.idle_for_s, Some(7_200));
        assert!(parsed.dry_run);
        assert!(parse_session_args("sessions", &args(&["--idle-for", "2h"])).is_err());
    }

    /// The remote lines, run by a real shell against a fake tmux: the
    /// listing reads back, the sweep kills only the exact fleet- name it
    /// was given, and a session a client attached to since the listing
    /// is kept.
    #[cfg(unix)]
    #[test]
    fn remote_lines_list_and_kill_only_exact_fleet_names_15422() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "fleet-ssh-sweep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let killed = dir.join("killed");
        let tmux = dir.join("tmux");
        std::fs::write(
            &tmux,
            format!(
                r#"#!/bin/sh
case "$1" in
  list-sessions) printf '0 1000 fleet-a\n0 1000 work\n1 1000 fleet-b\n' ;;
  display-message)
    case "$4" in
      =fleet-a) echo 0 ;;
      =fleet-b) echo 1 ;;
      *) exit 1 ;;
    esac ;;
  kill-session) printf '%s\n' "$3" >> '{}' ;;
esac
"#,
                killed.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&tmux).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.display());
        let run = |line: &str| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(line)
                .env("PATH", &path)
                .output()
                .unwrap();
            assert!(out.status.success(), "{line}");
            String::from_utf8(out.stdout).unwrap()
        };

        let (_, listed) = parse_listing(&run(&sessions_line())).unwrap();
        assert_eq!(
            listed,
            [
                session("fleet-a", 0, 1000),
                session("work", 0, 1000),
                session("fleet-b", 1, 1000)
            ]
        );
        // fleet-b had a client attach after the listing; `work` is
        // handed in to prove the line drops non-fleet names.
        let out = run(&sweep_line(&["fleet-a", "fleet-b", "work"]));
        assert_eq!(out, "swept fleet-a\n");
        assert_eq!(std::fs::read_to_string(&killed).unwrap(), "=fleet-a\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
}
