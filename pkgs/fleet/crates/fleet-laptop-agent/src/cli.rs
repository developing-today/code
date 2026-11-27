//! The `laptop-agent` domain's argument surface. Every option is declared
//! here and parsed strictly: anything unaccounted for is a usage error
//! (exit 2) before any transport is touched.

use std::sync::Arc;

use crate::{client, guard, legs};

pub const USAGE: &str = "usage: fleet laptop-agent <command>
  health
  jobs
  kill JOB
  exec [--cwd DIR] [--timeout SECS] [--shell] -- CMD...
  run  [--cwd DIR] [--timeout SECS] [--shell] -- CMD...
  poll JOB [--wait SECS]
  fetch REMOTE_PATH LOCAL_PATH";

pub const EXIT_USAGE: i32 = 2;

#[derive(Debug, PartialEq)]
pub enum Command {
    Health,
    Jobs,
    Kill {
        job: String,
    },
    Exec {
        cwd: Option<String>,
        timeout: u64,
        shell: bool,
        cmd: Vec<String>,
    },
    Run {
        cwd: Option<String>,
        shell: bool,
        cmd: Vec<String>,
    },
    Poll {
        job: String,
        wait: u64,
    },
    Fetch {
        remote: String,
        dest: String,
    },
}

/// A job id goes into a URL path, so only the characters the daemon
/// mints are accepted.
fn valid_job(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn number(flag: &str, raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|_| format!("{flag} expects a whole number of seconds, got {raw:?}"))
}

/// Split at the first `--`: the options before it, the command after it.
fn split_cmd(args: &[String]) -> (&[String], &[String]) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

/// Shared Python-compatible options for `exec` / `run`.
fn parse_exec_opts(opts: &[String]) -> Result<(Option<String>, u64, bool), String> {
    let mut cwd = None;
    let mut timeout = 120;
    let mut shell = false;
    let mut i = 0;
    while i < opts.len() {
        let arg = opts[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        match flag {
            "--shell" if inline.is_none() => shell = true,
            "--cwd" | "--timeout" => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        opts.get(i)
                            .cloned()
                            .ok_or_else(|| format!("{flag} needs a value"))?
                    }
                };
                if flag == "--cwd" {
                    cwd = Some(value);
                } else {
                    timeout = number(flag, &value)?;
                    if timeout == 0 {
                        return Err("--timeout must be positive".into());
                    }
                }
            }
            _ => return Err(format!("unknown option {arg:?}")),
        }
        i += 1;
    }
    Ok((cwd, timeout, shell))
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err("no command given".into());
    };
    match verb.as_str() {
        "health" if rest.is_empty() => Ok(Command::Health),
        "jobs" if rest.is_empty() => Ok(Command::Jobs),
        "kill" => match rest {
            [job] if valid_job(job) => Ok(Command::Kill { job: job.clone() }),
            _ => Err("kill takes one job id".into()),
        },
        "exec" | "run" => {
            let (opts, cmd) = split_cmd(rest);
            if cmd.is_empty() {
                return Err("no command given (use -- cmd...)".into());
            }
            let (cwd, timeout, shell) = parse_exec_opts(opts)?;
            if verb == "exec" {
                Ok(Command::Exec {
                    cwd,
                    timeout,
                    shell,
                    cmd: cmd.to_vec(),
                })
            } else {
                // `run --timeout` is accepted for Python-selector
                // compatibility but deliberately ignored: a detached job
                // submit carries no client-side timeout (legacy oracle:
                // laptop-agent-legacy.py submits cmd_spec only, with a
                // fixed 30 s submit deadline).
                let _ = timeout;
                Ok(Command::Run {
                    cwd,
                    shell,
                    cmd: cmd.to_vec(),
                })
            }
        }
        "poll" => {
            let (job, opts) = match rest.split_first() {
                Some((job, opts)) if valid_job(job) => (job, opts),
                _ => return Err("poll takes a job id".into()),
            };
            match opts {
                [] => Ok(Command::Poll {
                    job: job.clone(),
                    wait: 0,
                }),
                [flag, value] if flag == "--wait" => Ok(Command::Poll {
                    job: job.clone(),
                    wait: number("--wait", value)?,
                }),
                _ => Err("poll takes a job id and an optional --wait SECS".into()),
            }
        }
        "fetch" => match rest {
            [remote, dest] if !remote.is_empty() && !dest.is_empty() => Ok(Command::Fetch {
                remote: remote.clone(),
                dest: dest.clone(),
            }),
            _ => Err("fetch takes REMOTE_PATH and LOCAL_PATH".into()),
        },
        other => Err(format!("unknown command {other:?}")),
    }
}

fn command_words(shell: bool, cmd: &[String]) -> Vec<String> {
    if shell {
        cmd.join(" ")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    } else {
        cmd.to_vec()
    }
}

/// True when `args` asks for help: a help token before the first
/// `--` that is not the value of an option. This domain had no help
/// path at all — `poll --help` parsed `--help` as a job id and
/// polled it live (#13175). Help wins, the fleet-wide convention
/// (#10838); everything after `--` is the remote command, and an
/// option's value is the option's, even when it reads as a flag.
pub fn wants_help(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            break;
        }
        if matches!(a, "--help" | "-h" | "help") {
            return true;
        }
        if matches!(a, "--cwd" | "--timeout" | "--wait") {
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

/// Entry point for `fleet laptop-agent …`: returns the process exit code.
pub fn run(config: &fleet_config::Config, args: &[String]) -> i32 {
    if wants_help(args) {
        println!("{USAGE}");
        return 0;
    }
    let command = match parse(args) {
        Ok(c) => c,
        Err(message) => {
            eprintln!("laptop-agent: {message}\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let ladder = match legs::ladder(config) {
        Ok(l) => Arc::new(l),
        Err(message) => {
            eprintln!("laptop-agent: {message}");
            return 1;
        }
    };
    match command {
        Command::Health => client::health(&ladder),
        Command::Jobs => client::jobs(&ladder),
        Command::Kill { job } => client::kill(&ladder, &job),
        Command::Exec {
            cwd,
            timeout,
            shell,
            cmd,
        } => {
            if let Err(message) = guard::check_job(cwd.as_deref(), &command_words(shell, &cmd)) {
                eprintln!("laptop-agent: {message}");
                return 1;
            }
            let timeout = match std::env::var(crate::EXTERNAL_DEADLINE_ENV)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
            {
                Some(ext) => timeout.min((ext.max(1.0)) as u64),
                None => timeout,
            };
            let spec = client::command_spec(cwd.as_deref(), shell, &cmd);
            client::exec(&ladder, spec, timeout.max(1))
        }
        Command::Run { cwd, shell, cmd } => {
            if let Err(message) = guard::check_job(cwd.as_deref(), &command_words(shell, &cmd)) {
                eprintln!("laptop-agent: {message}");
                return 1;
            }
            let spec = client::command_spec(cwd.as_deref(), shell, &cmd);
            client::run(&ladder, spec)
        }
        Command::Poll { job, wait } => client::poll(&ladder, &job, wait),
        Command::Fetch { remote, dest } => {
            client::fetch(&ladder, &remote, std::path::Path::new(&dest))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn exec_parses_options_and_the_command_after_dashdash() {
        assert_eq!(
            parse(&v("exec --cwd /w --timeout 9 -- make test")).unwrap(),
            Command::Exec {
                cwd: Some("/w".into()),
                timeout: 9,
                shell: false,
                cmd: v("make test"),
            }
        );
        assert_eq!(
            parse(&v("exec --timeout=5 --shell -- echo hi")).unwrap(),
            Command::Exec {
                cwd: None,
                timeout: 5,
                shell: true,
                cmd: v("echo hi")
            }
        );
    }

    #[test]
    fn run_timeout_matches_python_legacy_run_submit_oracle() {
        // Oracle PythonLegacyRunSubmitOracle: laptop-agent-legacy.py
        // main() accepts `run --timeout N` (positive int, separated or
        // inline) but the submit path ignores it — the request is
        // POST /jobs with cmd_spec only and a fixed _set_deadline(30).
        // Every accepted spelling must therefore parse to the exact
        // Command::Run the no-timeout spelling produces.
        for timeout in [1, 5, 120, 3600, u64::MAX] {
            for option in [
                format!("--timeout {timeout}"),
                format!("--timeout={timeout}"),
            ] {
                for (flag, shell) in [("", false), ("--shell", true)] {
                    assert_eq!(
                        parse(&v(&format!(
                            "run {option} --cwd /w {flag} -- echo --timeout=0"
                        )))
                        .unwrap(),
                        Command::Run {
                            cwd: Some("/w".into()),
                            shell,
                            cmd: v("echo --timeout=0"),
                        },
                    );
                }
            }
        }
    }

    #[test]
    fn undeclared_options_are_usage_errors() {
        assert!(parse(&v("exec --bogus -- ls")).is_err());
        assert!(parse(&v("run --timeout 0 -- ls")).is_err());
        assert!(parse(&v("run --timeout -1 -- ls")).is_err());
        assert!(parse(&v("run --timeout x -- ls")).is_err());
        assert!(parse(&v("run --timeout -- ls")).is_err());
        assert!(parse(&v("exec --timeout x -- ls")).is_err());
        assert!(parse(&v("exec --timeout 0 -- ls")).is_err());
        assert!(parse(&v("exec --cwd")).is_err());
        assert!(parse(&v("exec --cwd /w")).is_err(), "a command is required");
    }

    #[test]
    fn job_ids_reach_the_path_only_when_well_formed() {
        assert!(parse(&v("kill job_1-2")).is_ok());
        assert!(parse(&v("kill ../etc")).is_err());
        assert!(parse(&v("poll a/b")).is_err());
        assert_eq!(
            parse(&v("poll job1 --wait 30")).unwrap(),
            Command::Poll {
                job: "job1".into(),
                wait: 30
            }
        );
        assert!(parse(&v("poll job1 --wait")).is_err());
    }

    #[test]
    fn help_tokens_win_before_any_transport() {
        // The #13175 shapes: per-verb help used to fall through to
        // live behaviour (a poll of a job named `--help`).
        assert!(wants_help(&v("poll --help")));
        assert!(wants_help(&v("--help")));
        assert!(wants_help(&v("health --help")));
        assert!(wants_help(&v("exec --help -- ls")));
        // Not help: an option's value, and the command after `--`.
        assert!(!wants_help(&v("exec --cwd --help -- ls")));
        assert!(!wants_help(&v("exec -- ls --help")));
        assert!(!wants_help(&v("poll job1 --wait 30")));
        assert!(!wants_help(&v("health")));
    }

    #[test]
    fn unknown_verbs_and_stray_arguments_are_refused() {
        assert!(parse(&v("frobnicate")).is_err());
        assert!(parse(&v("health now")).is_err());
        assert!(parse(&v("jobs --all")).is_err());
    }

    #[test]
    fn fetch_takes_exactly_two_paths() {
        assert_eq!(
            parse(&v("fetch /a/b.txt /tmp/b.txt")).unwrap(),
            Command::Fetch {
                remote: "/a/b.txt".into(),
                dest: "/tmp/b.txt".into()
            }
        );
        assert!(parse(&v("fetch /a")).is_err());
        assert!(parse(&v("fetch /a /b /c")).is_err());
    }
}
