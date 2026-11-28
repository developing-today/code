//! fleet-agent — one front door for any agent (design:
//! `doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/`).
//!
//! The fleet already has the legs: mailboxes on the message bridge
//! (`fleet bridge`), the queue (`fleet queue`), the laptop transport
//! ladder (`fleet laptop-agent`, `fleet proxy ssh`), placement and
//! secrets by name. What an agent of a new kind lacked was one place to
//! start: who am I here, what can this seat reach, what do I work on
//! next, and where do I build. This crate is that place. It adds no
//! transport of its own: every verb is a sequence of the existing verbs,
//! run through the same binary, so their auth, retries and fallbacks
//! apply unchanged.
//!
//! Verbs (`fleet agent <verb>`):
//!
//! * `kinds` — the agent kinds this fleet knows how to enrol.
//! * `enroll` — register a named agent of a kind and save its profile.
//! * `doctor` — which legs answer from this seat, and which intake
//!   rungs that leaves.
//! * `next` — the next piece of work, down the intake ladder:
//!   a command pushed to this agent, a queue claim, the local
//!   shortlist, then the highest-priority open issue. A rung that
//!   cannot answer is skipped and named, never fatal.
//! * `shortlist` — the local list `next` draws on: `status`, `add`,
//!   `take`, `fill`, `clear`.
//!
//! And one sibling domain, `fleet shell`: run a command or open a
//! shell where builds belong (the laptop, a codespace, or here).
//!
//! Nothing in this crate waits on a person. An issue parked "for the
//! owner" is handed out as ready work with a note saying so (FDR-0013).
//!
//! Exit codes, the workspace convention: 0 ok · 2 usage · 3 a leg
//! this verb needs is down · 5 no work on any rung (the queue's
//! no-claim code, so shift scripts need one check).

use std::path::PathBuf;

use fleet_config::Config;
use serde_json::Value;

pub mod enroll;
pub mod kinds;
pub mod next;
pub mod shell;
pub mod shortlist;

#[cfg(test)]
mod tests;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_DOWN: i32 = 3;
/// No rung had work. Equal to the queue client's no-claim exit.
pub const EXIT_NO_WORK: i32 = 5;

pub const USAGE: &str = "\
usage: fleet agent <verb> [options]

  kinds                         the agent kinds this fleet can enrol
  enroll --kind K --name N      register an agent and save its profile
         [--groups a,b] [--dry-run] [--profile PATH]
         [--bootstrap TOKEN]    provision from the owner's single-use
                                bootstrap token instead of the shared
                                fleet token (see fleet bridge exchange)
  doctor [--agent N]            which legs answer from this seat
  next   [--agent N]            the next piece of work, down the ladder:
         [--lane L] [--repo OWNER/NAME] [--low-water N]
         [--shortlist PATH] [--platform P] [--pool P]
         [--remaining-budget-minutes N] [--filter EXPR] [--queue Q]
         [--model-provider P] [--model-id M] [--model-variant V]
         [--no-push] [--no-queue] [--no-shortlist] [--no-priority]
  shortlist status|take|clear   the local list `next` draws on
  shortlist add --issue N --title T [--repo R] [--rank N] [--why TEXT]
  shortlist fill [--repo R] [--target N]

Every verb prints one JSON object. Exit: 0 ok, 2 usage, 3 a needed leg
is down, 5 no work on any rung.";

/// The shared tail of every per-verb usage: how output and exit
/// codes work, whatever the verb. It is USAGE's own tail, pinned by
/// test so the two cannot drift apart.
const USAGE_FOOTER: &str = "\
Every verb prints one JSON object. Exit: 0 ok, 2 usage, 3 a needed leg
is down, 5 no work on any rung.";

/// Per-verb usage for `fleet agent <verb> --help` (#13175): every
/// verb used to print the one shared USAGE, so a help request for a
/// single verb answered with all of them. Unknown verbs return
/// None and the caller falls back to the shared USAGE.
pub fn verb_usage(verb: &str) -> Option<String> {
    let section = match verb {
        "kinds" => {
            "\
usage: fleet agent kinds

The agent kinds this fleet can enrol."
        }
        "enroll" => {
            "\
usage: fleet agent enroll --kind K --name N [--groups a,b] [--dry-run] [--profile PATH]
       fleet agent enroll --kind K --name N --bootstrap TOKEN

Register an agent and save its profile. With --bootstrap, the
single-use token the owner minted for this agent is first exchanged
for the agent's own bridge credentials (stored in the vault by name
and recorded in the profile), and a Muse seat also gets its
bridge wake hook written."
        }
        "doctor" => {
            "\
usage: fleet agent doctor [--agent N]

Which legs answer from this seat."
        }
        "next" => {
            "\
usage: fleet agent next [--agent N] [--lane L] [--repo OWNER/NAME] [--low-water N]
       [--shortlist PATH] [--platform P] [--pool P]
       [--remaining-budget-minutes N] [--filter EXPR] [--queue Q]
       [--no-push] [--no-queue] [--no-shortlist] [--no-priority]

The next piece of work, down the ladder: pushed command, queue,
local shortlist, highest priority."
        }
        "shortlist" => {
            "\
usage: fleet agent shortlist status|take|clear
       fleet agent shortlist add --issue N --title T [--repo R] [--rank N] [--why TEXT]
       fleet agent shortlist fill [--repo R] [--target N]

The local list `next` draws on."
        }
        _ => return None,
    };
    Some(format!("{section}\n\n{USAGE_FOOTER}"))
}

/// A verb's failure: a usage error or a leg that is down.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentError {
    Usage(String),
    Down(String),
}

impl AgentError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AgentError::Usage(_) => EXIT_USAGE,
            AgentError::Down(_) => EXIT_DOWN,
        }
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Usage(m) => write!(f, "fleet agent: {m}"),
            AgentError::Down(m) => write!(f, "fleet agent: {m}"),
        }
    }
}

pub type R<T> = Result<T, AgentError>;

pub fn usage(msg: impl Into<String>) -> AgentError {
    AgentError::Usage(msg.into())
}

// ---- the outside world ----

/// What one sub-invocation answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    /// The stdout parsed as JSON, when it is JSON.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(self.stdout.trim()).ok()
    }
    /// One line saying why this call did not succeed, for a skip record.
    pub fn why(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        let line = text.lines().last().unwrap_or("").trim();
        let line: String = line.chars().take(200).collect();
        if line.is_empty() {
            format!("exit {}", self.code)
        } else {
            format!("exit {}: {line}", self.code)
        }
    }
}

/// Everything a verb needs from outside the process, so tests can
/// supply it. Production is [`RealWorld`].
pub trait World {
    /// Run this same fleet binary with `args`, capturing its output.
    /// Production bounds the call ([`step_limit`]): a leg that gives
    /// no answer in time is a leg that is down, never a hang.
    fn fleet(&self, args: &[String]) -> Out;
    /// Like [`World::fleet`], for the one kind of call that must not
    /// be cut short: a call that changes state on the far side (a
    /// queue claim), where stopping early would orphan the change.
    fn fleet_patient(&self, args: &[String]) -> Out {
        self.fleet(args)
    }
    /// Several independent fleet calls. Production runs them at the
    /// same time, so probing every leg of a seat with nothing
    /// reachable costs one limit, not one per leg.
    fn fleet_many(&self, calls: &[Vec<String>]) -> Vec<Out> {
        calls.iter().map(|c| self.fleet(c)).collect()
    }
    /// Run another program, capturing its output.
    fn program(&self, program: &str, args: &[String]) -> Out;
    /// Run a program attached to this terminal; its exit code.
    fn attach(&self, program: &str, args: &[String]) -> i32;
    fn env(&self, key: &str) -> Option<String>;
    /// Seconds since the Unix epoch.
    fn now(&self) -> u64;
    fn home(&self) -> Option<PathBuf>;
}

pub struct RealWorld;

/// Names the limit, in seconds, on one captured step.
pub const STEP_TIMEOUT_ENV: &str = "FLEET_AGENT_STEP_TIMEOUT";
/// Long enough for a healthy leg over a slow link, short enough that
/// a seat with nothing reachable still answers in under a minute.
pub const DEFAULT_STEP_TIMEOUT_SECS: u64 = 45;
/// A patient call gets this many step limits: past the secrets
/// chain's two-minute broker retry window at the default.
pub const PATIENT_FACTOR: u64 = 4;

/// The limit for one captured step from the variable's value: a
/// positive whole number of seconds, else the default.
pub fn step_limit(raw: Option<&str>) -> std::time::Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_STEP_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs)
}

fn env_step_limit() -> std::time::Duration {
    step_limit(std::env::var(STEP_TIMEOUT_ENV).ok().as_deref())
}

/// Run a command to completion or to `limit`, capturing its output.
/// A command still running at the limit is stopped and reported as
/// down (exit [`EXIT_DOWN`]) with a line saying how long it was given.
pub fn capture(mut cmd: std::process::Command, what: &str, limit: std::time::Duration) -> Out {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Out {
                code: 127,
                stdout: String::new(),
                stderr: format!("cannot run {what}: {e}"),
            };
        }
    };
    // Drain both pipes on threads so a chatty child never blocks on a
    // full pipe while this thread waits for it.
    fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::sync::mpsc::Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            let _ = tx.send(String::from_utf8_lossy(&text).into_owned());
        });
        rx
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= limit => break None,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    match status {
        Some(status) => {
            // The child is gone; whatever it wrote is already in the
            // pipes. A descendant that still holds a pipe open must
            // not hold this call past the child's exit for long.
            let grace = std::time::Duration::from_secs(2);
            Out {
                code: status.code().unwrap_or(1),
                stdout: stdout.recv_timeout(grace).unwrap_or_default(),
                stderr: stderr.recv_timeout(grace).unwrap_or_default(),
            }
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Out {
                code: EXIT_DOWN,
                stdout: String::new(),
                stderr: format!("{what} gave no answer within {}s", limit.as_secs()),
            }
        }
    }
}

fn this_fleet(args: &[String], limit: std::time::Duration) -> Out {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return Out {
                code: 127,
                stdout: String::new(),
                stderr: format!("cannot find this fleet binary: {e}"),
            };
        }
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    let what = format!(
        "fleet {}",
        args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
    );
    capture(cmd, what.trim_end(), limit)
}

impl World for RealWorld {
    fn fleet(&self, args: &[String]) -> Out {
        this_fleet(args, env_step_limit())
    }
    fn fleet_patient(&self, args: &[String]) -> Out {
        this_fleet(args, env_step_limit() * PATIENT_FACTOR as u32)
    }
    fn fleet_many(&self, calls: &[Vec<String>]) -> Vec<Out> {
        let limit = env_step_limit();
        std::thread::scope(|scope| {
            let running: Vec<_> = calls
                .iter()
                .map(|call| scope.spawn(move || this_fleet(call, limit)))
                .collect();
            running
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| Out {
                        code: EXIT_DOWN,
                        stdout: String::new(),
                        stderr: "the probe did not finish".to_string(),
                    })
                })
                .collect()
        })
    }
    fn program(&self, program: &str, args: &[String]) -> Out {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        capture(cmd, program, env_step_limit())
    }
    fn attach(&self, program: &str, args: &[String]) -> i32 {
        let target = if program == "fleet" {
            match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("fleet: cannot find this fleet binary: {e}");
                    return 127;
                }
            }
        } else {
            PathBuf::from(program)
        };
        match std::process::Command::new(target).args(args).status() {
            Ok(s) => s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("fleet: cannot run {program}: {e}");
                127
            }
        }
    }
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    }
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
    fn home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    }
}

/// Build an argv from string slices.
pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

// ---- argument parsing ----

/// Parsed options: `--name value`, `--name=value`, switches, and the
/// words after `--`. An unknown option is a usage error, never ignored.
#[derive(Debug, Default)]
pub struct Args {
    flags: Vec<(String, Option<String>)>,
    pub positional: Vec<String>,
    /// The words after a `--` separator, verbatim.
    pub tail: Vec<String>,
}

pub fn parse(args: &[String], switches: &[&str], values: &[&str]) -> R<Args> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            out.tail = it.cloned().collect();
            break;
        }
        if !a.starts_with("--") {
            out.positional.push(a.clone());
            continue;
        }
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        if values.contains(&name.as_str()) {
            // A present option with no value is a usage error, never
            // the default (the operator argument contract): a blank
            // value, the end of the line, `--`, or another declared
            // option where the value should be.
            let declared = |token: &str| {
                let head = token.split_once('=').map_or(token, |(n, _)| n);
                token == "--" || values.contains(&head) || switches.contains(&head)
            };
            let v = match inline {
                Some(v) => v,
                None => match it.next() {
                    Some(v) if !declared(v) => v.clone(),
                    _ => return Err(usage(format!("{name} needs a value"))),
                },
            };
            if v.trim().is_empty() {
                return Err(usage(format!("{name} needs a value")));
            }
            out.flags.push((name, Some(v)));
        } else if switches.contains(&name.as_str()) {
            if inline.is_some() {
                return Err(usage(format!("{name} takes no value")));
            }
            out.flags.push((name, None));
        } else {
            return Err(usage(format!("unknown option {name}")));
        }
    }
    Ok(out)
}

impl Args {
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
    pub fn last(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.clone())
            .map(|v| v.trim().to_string())
    }
    pub fn uint(&self, name: &str) -> R<Option<u64>> {
        match self.last(name) {
            None => Ok(None),
            Some(v) => v
                .parse::<u64>()
                .map(Some)
                .map_err(|_| usage(format!("{name} must be a whole number"))),
        }
    }
    pub fn no_positional(&self, verb: &str) -> R<()> {
        if let Some(p) = self.positional.first() {
            return Err(usage(format!("{verb} takes no argument '{p}'")));
        }
        if !self.tail.is_empty() {
            return Err(usage(format!("{verb} takes nothing after --")));
        }
        Ok(())
    }
}

/// An agent or lane name: what the bridge and the queue accept.
pub fn check_name(what: &str, name: &str) -> R<String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(name.to_string())
    } else {
        Err(usage(format!(
            "{what} '{name}' must be 1-64 of lowercase letters, digits, '.', '_' or '-', \
             starting with a letter or digit"
        )))
    }
}

/// `owner/name`, the only repository shape the fleet uses.
pub fn check_repo(repo: &str) -> R<String> {
    let ok = match repo.split_once('/') {
        Some((owner, name)) => {
            let part = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            };
            part(owner) && part(name)
        }
        None => false,
    };
    if ok {
        Ok(repo.to_string())
    } else {
        Err(usage(format!("--repo '{repo}' must be owner/name")))
    }
}

pub const DEFAULT_REPO: &str = "developing-today/code";

// ---- domain entry points ----

fn print(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

fn wants_help(args: &[String]) -> bool {
    args.iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--help" || a == "-h" || a == "help")
}

/// Runs one `fleet agent` verb; Ok is the JSON to print and the exit code.
pub fn execute(world: &dyn World, args: &[String]) -> R<(Value, i32)> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(usage("missing verb (try: fleet agent --help)"));
    };
    match verb.as_str() {
        "kinds" => {
            parse(rest, &[], &[])?.no_positional("kinds")?;
            Ok((kinds::table(), EXIT_OK))
        }
        "enroll" => enroll::enroll(world, rest),
        "doctor" => enroll::doctor(world, rest),
        "next" => next::next(world, rest),
        "shortlist" => shortlist::run(world, rest),
        other => Err(usage(format!(
            "unknown verb '{other}' (try: fleet agent --help)"
        ))),
    }
}

/// The domain entry point: `fleet agent <verb> ...`.
pub fn run(config: &Config, args: &[String]) -> i32 {
    if args.is_empty() {
        println!("{USAGE}");
        return EXIT_USAGE;
    }
    if wants_help(args) {
        match args.first().and_then(|verb| verb_usage(verb)) {
            Some(text) => println!("{text}"),
            None => println!("{USAGE}"),
        }
        return EXIT_OK;
    }
    // The sub-invocations load fleet config themselves; this domain
    // has no settings of its own yet.
    let _ = config;
    match execute(&RealWorld, args) {
        Ok((v, code)) => {
            print(&v);
            code
        }
        Err(e) => {
            eprintln!("{e}");
            e.exit_code()
        }
    }
}

/// The sibling domain's entry point: `fleet shell ...`.
pub fn run_shell(config: &Config, args: &[String]) -> i32 {
    if wants_help(args) {
        println!("{}", shell::USAGE);
        return EXIT_OK;
    }
    match shell::run(config, &RealWorld, args) {
        Ok(shell::Done::Printed(v)) => {
            print(&v);
            EXIT_OK
        }
        Ok(shell::Done::PrintedCode(v, code)) => {
            print(&v);
            code
        }
        Ok(shell::Done::Ran(code)) => code,
        Err(e) => {
            eprintln!(
                "{}",
                e.to_string().replacen("fleet agent:", "fleet shell:", 1)
            );
            e.exit_code()
        }
    }
}
