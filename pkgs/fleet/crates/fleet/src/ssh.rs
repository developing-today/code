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

use std::path::PathBuf;
use std::process::Command;

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
        .arg(format!(
            "ConnectTimeout={}",
            connect_timeout_s.clamp(1, 60)
        ))
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
    let mut line = cmd.join(" ");
    if let Some(d) = cwd {
        line = format!("cd {} && {}", shell_quote(d), line);
    }
    let mut ssh = ssh_base(&home, timeout_s);
    ssh.arg(line);
    spawn_with_deadline(&mut ssh, timeout_s)
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
            cwd = Some(
                args.get(i)
                    .ok_or("exec: --cwd needs a value")?
                    .to_string(),
            );
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
            return Err(format!("exec: unexpected argument {a} (command goes after --)"));
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
