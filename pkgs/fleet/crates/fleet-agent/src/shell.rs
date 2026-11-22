//! `fleet shell` — run a command, or open a shell, where builds belong.
//!
//! A limited sandbox is the wrong place to compile. This verb picks a
//! better one and gets out of the way: the fleet laptop (through the
//! laptop-agent transport ladder: its broker, the Cloudflare tunnel,
//! the tailnet, then ssh), a codespace, or this machine when this
//! machine is already the right place. It adds no transport; it builds
//! the argv for the verb that already knows how to get there.
//!
//! `auto` (the default) resolves in this order: `FLEET_SHELL_TARGET`;
//! here, when this seat is the laptop; the laptop, when it answers; a
//! codespace, when one exists; here, saying that nothing remote
//! answered. An explicit `--target` is used as given and never falls
//! back, so a caller that asked for the laptop does not silently build
//! in a sandbox.

use fleet_config::Config;
use serde_json::{Value, json};

use crate::{AgentError, R, World, argv, parse, usage};

pub const USAGE: &str = "\
usage: fleet shell [options] [-- COMMAND...]
       fleet shell targets

Run COMMAND where builds belong, or open a shell there with no COMMAND.

  --target auto|laptop|codespace|local   where (default: auto)
  --codespace NAME                       which codespace (default: the first available)
  --cwd DIR                              directory to run in
  --build                                a heavy build: take a fleet-build-lock slot;
                                          direct Cargo commands use its shared target lane
  --timeout SECS                         how long a laptop command may run
  --detach                               start it as a laptop job and return its id
                                         (follow with: fleet laptop-agent poll JOB --wait N)
  --print                                print the plan as JSON instead of running it

auto resolves: FLEET_SHELL_TARGET, then here if this seat is the laptop, then the
laptop if it answers, then a codespace, then here.";

pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const BUILD_TIMEOUT_SECS: u64 = 3600;
const BUILD_LOCK: &str = "~/tools/fleet-build-lock";

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Local,
    Laptop,
    Codespace(String),
}

impl Target {
    pub fn id(&self) -> &'static str {
        match self {
            Target::Local => "local",
            Target::Laptop => "laptop",
            Target::Codespace(_) => "codespace",
        }
    }
}

/// What will be run: `fleet <args>` when `program` is "fleet".
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub target: Target,
    pub program: String,
    pub args: Vec<String>,
    /// Why this target, in one line.
    pub because: String,
}

impl Plan {
    pub fn to_json(&self) -> Value {
        json!({
            "target": self.target.id(),
            "codespace": match &self.target { Target::Codespace(n) => Some(n.clone()), _ => None },
            "program": self.program,
            "args": self.args,
            "because": self.because,
        })
    }
}

pub enum Done {
    Printed(Value),
    Ran(i32),
}

/// Quote one word for a POSIX shell, only when it needs it. `~` is
/// never left bare: a shell expands a leading one to a home directory.
pub fn sh_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | ',' | '+' | '@' | '%')
        });
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// One word in single quotes, whatever it holds.
pub fn sh_quote_always(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The command as one shell string. A single word is taken as a shell
/// string already; several words are quoted one by one.
pub fn shell_string(words: &[String]) -> String {
    match words {
        [one] => one.clone(),
        many => many
            .iter()
            .map(|w| sh_quote(w))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn is_laptop_seat(config: &Config, world: &dyn World) -> bool {
    world
        .env("FLEET_LOCATION")
        .or_else(|| config.host.location.clone())
        .is_some_and(|l| l.trim() == "laptop")
}

fn laptop_answers(world: &dyn World) -> bool {
    world.fleet(&argv(&["laptop-agent", "health"])).ok()
}

/// The first codespace `gh` lists, preferring one that is running.
fn first_codespace(world: &dyn World) -> Option<String> {
    let out = world.program(
        "gh",
        &argv(&["codespace", "list", "--json", "name,repository,state"]),
    );
    let list = out.json()?;
    let items = list.as_array()?;
    let name_of = |v: &Value| v.get("name").and_then(Value::as_str).map(str::to_string);
    items
        .iter()
        .find(|c| c.get("state").and_then(Value::as_str) == Some("Available"))
        .and_then(name_of)
        .or_else(|| items.first().and_then(name_of))
}

fn parse_target(word: &str) -> R<&'static str> {
    match word {
        "auto" => Ok("auto"),
        "local" => Ok("local"),
        "laptop" => Ok("laptop"),
        "codespace" => Ok("codespace"),
        other => Err(usage(format!(
            "unknown target '{other}': auto, laptop, codespace or local"
        ))),
    }
}

/// Decide where. An explicit target is used as given; only `auto` falls back.
pub fn resolve(
    config: &Config,
    world: &dyn World,
    asked: &str,
    codespace: Option<String>,
) -> R<(Target, String)> {
    let from_env = world.env("FLEET_SHELL_TARGET");
    let (want, source) = match (asked, &from_env) {
        ("auto", Some(env)) => (parse_target(env.trim())?, "FLEET_SHELL_TARGET"),
        _ => (parse_target(asked)?, "--target"),
    };
    match want {
        "local" => Ok((Target::Local, format!("{source} says local"))),
        "laptop" => Ok((Target::Laptop, format!("{source} says laptop"))),
        "codespace" => {
            let name = codespace.or_else(|| first_codespace(world)).ok_or_else(|| {
                AgentError::Down(
                    "no codespace to use: `gh codespace list` returned none (create one, or pass --codespace NAME)"
                        .to_string(),
                )
            })?;
            Ok((Target::Codespace(name), format!("{source} says codespace")))
        }
        _ => {
            if is_laptop_seat(config, world) {
                return Ok((Target::Local, "this seat is the laptop".to_string()));
            }
            if laptop_answers(world) {
                return Ok((Target::Laptop, "the laptop answers".to_string()));
            }
            if let Some(name) = codespace.or_else(|| first_codespace(world)) {
                return Ok((
                    Target::Codespace(name),
                    "the laptop did not answer; a codespace exists".to_string(),
                ));
            }
            Ok((
                Target::Local,
                "neither the laptop nor a codespace answered; running here".to_string(),
            ))
        }
    }
}

pub struct Options {
    pub cwd: Option<String>,
    pub build: bool,
    pub timeout: Option<u64>,
    pub detach: bool,
}

/// Whether a command string needs shell control flow rather than being
/// a command plus arguments. Plain Cargo strings must reach the build-lock
/// wrapper as fleet-build-lock cargo ..., so its target broker can put
/// them in the shared lane. Compound commands remain one shell invocation.
fn needs_shell(command: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                }
                continue;
            }
            Some('"') => {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quote = None;
                }
                continue;
            }
            _ => {}
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if matches!(
            ch,
            ';' | '|' | '&' | '<' | '>' | '\x60' | '(' | ')' | '\n' | '\r'
        ) {
            return true;
        }
    }

    let first = command
        .trim_start()
        .split_ascii_whitespace()
        .next()
        .unwrap_or("");
    if first.contains('=') {
        return true;
    }
    matches!(
        first,
        "cd" | "pushd"
            | "popd"
            | "export"
            | "unset"
            | "source"
            | "."
            | "eval"
            | "exec"
            | "set"
            | "umask"
            | "trap"
            | "alias"
            | "read"
            | "return"
            | "break"
            | "continue"
            | "if"
            | "for"
            | "while"
            | "until"
            | "case"
            | "select"
            | "time"
            | "!"
    )
}

/// Run direct commands as wrapper argv so Cargo reaches the broker's shared
/// target lane. Keep compound shell input inside one sh -c argument.
fn under_build_lock(command: &str) -> String {
    if needs_shell(command) {
        format!("{BUILD_LOCK} sh -c {}", sh_quote_always(command))
    } else {
        format!("{BUILD_LOCK} {command}")
    }
}

/// A build on this host: under the build lock where this host has one
/// (the laptop does), and plainly, with a note, where it does not.
fn local_build_line(command: &str, cwd: Option<&str>) -> String {
    let change_dir = cwd
        .map(|dir| format!("cd {} || exit $?; ", sh_quote(dir)))
        .unwrap_or_default();
    let (locked, fallback) = if needs_shell(command) {
        let quoted = sh_quote_always(command);
        (
            format!("{BUILD_LOCK} sh -c {quoted}"),
            format!("sh -c {quoted}"),
        )
    } else {
        (format!("{BUILD_LOCK} {command}"), command.to_string())
    };
    format!(
        "{change_dir}if [ -x {BUILD_LOCK} ]; then exec {locked}; fi; echo 'fleet shell: this host has no build lock; running without one' >&2; exec {fallback}"
    )
}

/// Build the argv for a target. Pure: no probing, no running.
pub fn plan(target: Target, because: String, words: &[String], opts: &Options) -> R<Plan> {
    let interactive = words.is_empty();
    if interactive && (opts.build || opts.detach || opts.timeout.is_some()) {
        return Err(usage(
            "--build, --detach and --timeout need a COMMAND after --",
        ));
    }
    let command = shell_string(words);
    let (program, args) = match &target {
        Target::Local => {
            if interactive {
                ("sh".to_string(), argv(&["-l"]))
            } else {
                if opts.detach {
                    return Err(usage(
                        "--detach starts a laptop job; it needs --target laptop",
                    ));
                }
                let line = if opts.build {
                    local_build_line(&command, opts.cwd.as_deref())
                } else {
                    match &opts.cwd {
                        Some(dir) => format!("cd {} && {command}", sh_quote(dir)),
                        None => command,
                    }
                };
                ("sh".to_string(), vec!["-lc".to_string(), line])
            }
        }
        Target::Laptop => {
            if interactive {
                // The pinned ssh leaf: destination and keys are fleet's.
                ("fleet".to_string(), argv(&["proxy", "ssh", "--"]))
            } else {
                let mut args = argv(&["laptop-agent", if opts.detach { "run" } else { "exec" }]);
                if let Some(dir) = &opts.cwd {
                    args.push("--cwd".to_string());
                    args.push(dir.clone());
                }
                if !opts.detach {
                    let timeout = opts.timeout.unwrap_or(if opts.build {
                        BUILD_TIMEOUT_SECS
                    } else {
                        DEFAULT_TIMEOUT_SECS
                    });
                    args.push("--timeout".to_string());
                    args.push(timeout.to_string());
                }
                args.push("--shell".to_string());
                args.push("--".to_string());
                args.push(if opts.build {
                    under_build_lock(&command)
                } else {
                    command
                });
                ("fleet".to_string(), args)
            }
        }
        Target::Codespace(name) => {
            if opts.detach {
                return Err(usage(
                    "--detach starts a laptop job; it needs --target laptop",
                ));
            }
            let mut args = argv(&["codespace", "ssh", "-c", name]);
            if !interactive {
                let line = match &opts.cwd {
                    Some(dir) => format!("cd {} && {command}", sh_quote(dir)),
                    None => command,
                };
                args.push("--".to_string());
                args.push(line);
            }
            ("gh".to_string(), args)
        }
    };
    Ok(Plan {
        target,
        program,
        args,
        because,
    })
}

fn targets(config: &Config, world: &dyn World) -> Value {
    let laptop_seat = is_laptop_seat(config, world);
    let laptop = laptop_seat || laptop_answers(world);
    let codespace = first_codespace(world);
    json!({
        "targets": [
            {"target": "laptop", "reachable": laptop,
             "how": if laptop_seat { "this seat is the laptop" } else { "fleet laptop-agent (broker, Cloudflare tunnel, tailnet, ssh)" }},
            {"target": "codespace", "reachable": codespace.is_some(), "codespace": codespace,
             "how": "gh codespace ssh"},
            {"target": "local", "reachable": true, "how": "this machine"},
        ],
        "auto_would_pick": resolve(config, world, "auto", None).map(|(t, why)| json!({"target": t.id(), "because": why})).unwrap_or(Value::Null),
    })
}

/// `fleet shell ...`.
pub fn run(config: &Config, world: &dyn World, args: &[String]) -> R<Done> {
    let a = parse(
        args,
        &["--build", "--detach", "--print"],
        &["--target", "--codespace", "--cwd", "--timeout"],
    )?;
    match a.positional.as_slice() {
        [] => {}
        [one] if one == "targets" => {
            if !a.tail.is_empty() {
                return Err(usage("targets takes nothing after --"));
            }
            return Ok(Done::Printed(targets(config, world)));
        }
        [other, ..] => {
            return Err(usage(format!(
                "unexpected argument '{other}': put the command after `--`"
            )));
        }
    }
    let asked = a.last("--target").unwrap_or_else(|| "auto".to_string());
    let (target, because) = resolve(config, world, &asked, a.last("--codespace"))?;
    let opts = Options {
        cwd: a.last("--cwd"),
        build: a.has("--build"),
        timeout: a.uint("--timeout")?,
        detach: a.has("--detach"),
    };
    let plan = plan(target, because, &a.tail, &opts)?;
    if a.has("--print") {
        return Ok(Done::Printed(plan.to_json()));
    }
    Ok(Done::Ran(world.attach(&plan.program, &plan.args)))
}
