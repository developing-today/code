//! `fleet shell` — run a command, or open a shell, where builds belong.
//!
//! A limited sandbox is the wrong place to compile. This verb picks a
//! better one and gets out of the way: the fleet laptop (through the
//! laptop-agent transport ladder: its broker, the Cloudflare tunnel,
//! the tailnet, then ssh), a codespace, a GitHub Actions runner
//! (through placement: `fleet queue dispatch`, then the session is
//! polled to its result), or this machine when this machine is
//! already the right place. It adds no transport; it builds the argv
//! for the verb that already knows how to get there.
//!
//! `auto` (the default) resolves in this order: `FLEET_SHELL_TARGET`;
//! here, when this seat is the laptop; the laptop, when it answers; a
//! codespace, when one exists; here, saying that nothing remote
//! answered. An explicit `--target` is used as given and never falls
//! back, so a caller that asked for the laptop does not silently build
//! in a sandbox.

use fleet_config::Config;
use serde_json::{Value, json};

use crate::{AgentError, R, World, argv, check_repo, parse, usage};

pub const USAGE: &str = "\
usage: fleet shell [options] [-- COMMAND...]
       fleet shell targets

Run COMMAND where builds belong, or open a shell there with no COMMAND.

  --target auto|laptop|codespace|actions|local   where (default: auto)
  --codespace NAME                       which codespace (default: the first available)
  --repo OWNER/NAME                      the repository an actions run executes
                                         (default: this checkout's origin)
  --cwd DIR                              directory to run in (Cargo workspace for shared lane)
  --build                                a heavy build: take a fleet-build-lock slot;
                                          Cargo workspaces use a shared target lane
  --timeout SECS                         how long a laptop command may run; on the
                                         actions target, how long to wait for the run
  --detach                               start it as a laptop job and return its id
                                         (follow with: fleet laptop-agent poll JOB --wait N);
                                         on the actions target, dispatch and return the
                                         session handle instead of waiting
  --print                                print the plan as JSON instead of running it

auto resolves: FLEET_SHELL_TARGET, then here if this seat is the laptop, then the
laptop if it answers, then a codespace, then here. auto never picks actions.

The actions target dispatches COMMAND through placement (fleet queue dispatch)
to a registered GitHub Actions surface and waits for the session result: the
exit code is 0 when the run succeeded and 1 when it failed or was cancelled,
and the printed record carries the run URL — the run's own output lives on
that page (gh run view --log). Reconnect to a run you stopped waiting on with:
fleet queue session result HANDLE";

pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const BUILD_TIMEOUT_SECS: u64 = 3600;
const BUILD_LOCK: &str = "~/tools/fleet-build-lock";

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Local,
    Laptop,
    Codespace(String),
    /// A GitHub Actions run of the repository, through placement.
    Actions(String),
}

impl Target {
    pub fn id(&self) -> &'static str {
        match self {
            Target::Local => "local",
            Target::Laptop => "laptop",
            Target::Codespace(_) => "codespace",
            Target::Actions(_) => "actions",
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
            "repo": match &self.target { Target::Actions(r) => Some(r.clone()), _ => None },
            "program": self.program,
            "args": self.args,
            "because": self.because,
        })
    }
}

pub enum Done {
    Printed(Value),
    /// Print this record, then exit with this code: the actions
    /// target's verdict, where the code is the run's pass/fail.
    PrintedCode(Value, i32),
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
        "actions" => Ok("actions"),
        other => Err(usage(format!(
            "unknown target '{other}': auto, laptop, codespace, actions or local"
        ))),
    }
}

/// The `owner/name` a git remote URL points at, for the shapes git
/// and GitHub actually use; None for anything else.
fn repo_from_remote(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = url
        .rsplit_once("github.com/")
        .map(|(_, rest)| rest)
        .or_else(|| url.rsplit_once("github.com:").map(|(_, rest)| rest))?;
    check_repo(path).ok()
}

/// The repository an actions run executes: the caller's `--repo`,
/// else this checkout's origin remote.
fn actions_repo(world: &dyn World, asked: Option<String>) -> R<String> {
    if let Some(repo) = asked {
        return check_repo(&repo);
    }
    let out = world.program("git", &argv(&["remote", "get-url", "origin"]));
    if out.ok() {
        if let Some(repo) = repo_from_remote(out.stdout.trim()) {
            return Ok(repo);
        }
    }
    Err(AgentError::Down(
        "the actions target needs a repository: pass --repo OWNER/NAME \
         (or run inside a checkout whose origin is on GitHub)"
            .to_string(),
    ))
}

/// Decide where. An explicit target is used as given; only `auto` falls back.
pub fn resolve(
    config: &Config,
    world: &dyn World,
    asked: &str,
    codespace: Option<String>,
    repo: Option<String>,
) -> R<(Target, String)> {
    let from_env = world.env("FLEET_SHELL_TARGET");
    let (want, source) = match (asked, &from_env) {
        ("auto", Some(env)) => (parse_target(env.trim())?, "FLEET_SHELL_TARGET"),
        _ => (parse_target(asked)?, "--target"),
    };
    match want {
        "local" => Ok((Target::Local, format!("{source} says local"))),
        "laptop" => Ok((Target::Laptop, format!("{source} says laptop"))),
        "actions" => {
            let repo = actions_repo(world, repo)?;
            Ok((Target::Actions(repo), format!("{source} says actions")))
        }
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

/// Commands that compile through Cargo use the target broker. "just" and
/// "make" stay conservative because their recipes can hide Cargo builds.
fn may_invoke_cargo(command: &str) -> bool {
    const COMPILE_ACTIONS: &[&str] = &[
        "build", "check", "test", "bench", "clippy", "run", "install", "doc", "rustc", "nextest",
        "fix", "package", "publish",
    ];
    const CARGO_FLAGS: &[&str] = &[
        "--locked",
        "--offline",
        "--frozen",
        "--quiet",
        "--verbose",
        "-q",
        "-v",
    ];
    let words: Vec<_> = command
        .split(|ch: char| {
            !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '.' | '=' | '/'))
        })
        .filter(|word| !word.is_empty())
        .collect();

    for (index, word) in words.iter().enumerate() {
        if matches!(*word, "just" | "make") {
            return true;
        }
        if *word != "cargo" && !word.ends_with("/cargo") {
            continue;
        }

        let mut next = index + 1;
        while let Some(arg) = words.get(next).copied() {
            if matches!(arg, "--config" | "--color" | "-Z" | "--manifest-path") {
                next += 2;
                continue;
            }
            if arg.starts_with('+')
                || arg.starts_with("-Z")
                || arg.starts_with("-v")
                || CARGO_FLAGS.contains(&arg)
                || arg.starts_with("--config=")
                || arg.starts_with("--color=")
                || arg.starts_with("--manifest-path=")
            {
                next += 1;
                continue;
            }
            if COMPILE_ACTIONS.contains(&arg) {
                return true;
            }
            // A non-compiling Cargo subcommand must not hide a later build
            // in the same shell command (for example fmt && test).
            break;
        }
    }
    false
}

/// Keep the whole shell command under one build-lock slot while leasing a
/// shared target directory for its Cargo work. The caller's cwd matters:
/// laptop-agent applies --cwd before starting this command, and the broker
/// cleans only that workspace's artifacts.
fn broker_aware_shell(command: &str) -> String {
    let profile = if command
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .any(|word| matches!(word, "test" | "bench"))
    {
        "test"
    } else if command
        .split_ascii_whitespace()
        .any(|word| word == "--release")
        || (command
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
            .any(|word| word == "install")
            && !command
                .split_ascii_whitespace()
                .any(|word| word == "--debug"))
    {
        // cargo install uses the release profile by default.
        "release"
    } else {
        "dev"
    };
    let script = r#"command_text=__COMMAND__
broker="$HOME/tools/cargo-target-broker"
if [ -z "$CARGO_TARGET_DIR" ]; then
  manifest_arg=$(python3 -c '
import shlex, sys
try:
    words = shlex.split(sys.argv[1])
except ValueError:
    raise SystemExit(0)
separators = {";", "&&", "||", "|", "&"}
for index, word in enumerate(words):
    if word != "cargo" and not word.endswith("/cargo"):
        continue
    index += 1
    while index < len(words) and words[index] not in separators:
        token = words[index]
        if token == "--manifest-path" and index + 1 < len(words):
            print(words[index + 1])
            raise SystemExit(0)
        if token.startswith("--manifest-path="):
            print(token.split("=", 1)[1])
            raise SystemExit(0)
        if token == "--path" and index + 1 < len(words):
            source = words[index + 1]
            print(source if source.endswith("Cargo.toml") else source.rstrip("/") + "/Cargo.toml")
            raise SystemExit(0)
        if token.startswith("--path="):
            source = token.split("=", 1)[1]
            print(source if source.endswith("Cargo.toml") else source.rstrip("/") + "/Cargo.toml")
            raise SystemExit(0)
        index += 1
' "$command_text" 2>/dev/null)
  if [ -n "$manifest_arg" ]; then
    case "$manifest_arg" in
      /*) manifest_path=$manifest_arg ;;
      *) manifest_path="$PWD/$manifest_arg" ;;
    esac
    manifest=$(cargo locate-project --workspace --message-format plain --manifest-path "$manifest_path" 2>/dev/null || true)
  else
    manifest=$(cargo locate-project --workspace --message-format plain 2>/dev/null || true)
  fi
  if [ -n "$manifest" ]; then
    root=$(dirname -- "$manifest")
    workspace_real=$(cd "$root" 2>/dev/null && pwd -P) || workspace_real=""
    cwd_real=$(pwd -P)
    repo=$(printf '%s\n' "$root" | sed -n 's#^.*/pkgs/\([^/]*\).*#\1#p')
    if [ -z "$repo" ]; then repo=$(basename -- "$root"); fi
    repo=$(printf '%s' "$repo" | tr -c 'A-Za-z0-9._-' '_')
    if [ -n "$repo" ]; then
      if [ -x "$broker" ]; then
        case "$cwd_real/" in
          "$workspace_real/"*) outside_workspace=false ;;
          *) outside_workspace=true ;;
        esac
        if [ "$outside_workspace" = true ]; then
          lease=$("$broker" checkout --repo "$repo" --profile __PROFILE__ --workspace-cwd "$workspace_real" --wait 900 --json 2>/dev/null) || lease=""
        else
          lease=$("$broker" checkout --repo "$repo" --profile __PROFILE__ --wait 900 --json 2>/dev/null) || lease=""
        fi
        if [ -n "$lease" ]; then
          target=$(printf '%s' "$lease" | python3 -c 'import json,sys; print(json.load(sys.stdin)["path"])' 2>/dev/null) || target=""
          lease_id=$(printf '%s' "$lease" | python3 -c 'import json,sys; print(json.load(sys.stdin)["lease"])' 2>/dev/null) || lease_id=""
          if [ -n "$target" ] && [ -n "$lease_id" ]; then
            export CARGO_TARGET_DIR="$target"
            worker_pid=""
            lane_interrupted=false
            tmp_root="$TMPDIR"
            if [ -z "$tmp_root" ]; then tmp_root=/tmp; fi
            group_file=$(mktemp "$tmp_root/fleet-cargo-group.XXXXXX" 2>/dev/null) || group_file=""
            cleanup_lane() {
              status=$?
              trap - EXIT HUP INT TERM
              if [ "$lane_interrupted" = true ] && [ -n "$group_file" ] && [ ! -s "$group_file" ]; then
                attempts=0
                while [ "$attempts" -lt 10 ] && [ ! -s "$group_file" ]; do
                  sleep 0.05
                  attempts=$((attempts + 1))
                done
              fi
              if [ -n "$group_file" ] && [ -s "$group_file" ]; then
                worker_pgid=$(cat "$group_file" 2>/dev/null || :)
                case "$worker_pgid" in
                  ''|*[!0-9]*) ;;
                  *)
                    if [ "$lane_interrupted" = true ]; then
                      # laptop-agent escalates the outer process group
                      # after 2 seconds; kill this detached group first.
                      grace=0
                      while kill -0 "-$worker_pgid" 2>/dev/null && [ "$grace" -lt 10 ]; do
                        sleep 0.1
                        grace=$((grace + 1))
                      done
                      if kill -0 "-$worker_pgid" 2>/dev/null; then
                        echo "fleet shell: terminating unresponsive build process group" >&2
                        kill -KILL "-$worker_pgid" 2>/dev/null || :
                        if [ -n "$worker_pid" ]; then kill -KILL "$worker_pid" 2>/dev/null || :; fi
                      fi
                    fi
                    if [ -n "$worker_pid" ]; then
                      wait "$worker_pid" >/dev/null 2>&1 || :
                    fi
                    while kill -0 "-$worker_pgid" 2>/dev/null; do sleep 0.05; done
                    ;;
                esac
              elif [ -n "$worker_pid" ]; then
                wait "$worker_pid" >/dev/null 2>&1 || :
              fi
              if [ -n "$group_file" ]; then rm -f "$group_file"; fi
              "$broker" checkin --lease "$lease_id" >/dev/null 2>&1 || :
              exit "$status"
            }
            interrupt_lane() {
              signal=$1
              code=$2
              lane_interrupted=true
              if [ -n "$group_file" ] && [ ! -s "$group_file" ]; then
                attempts=0
                while [ "$attempts" -lt 10 ] && [ ! -s "$group_file" ]; do
                  sleep 0.05
                  attempts=$((attempts + 1))
                done
              fi
              if [ -n "$group_file" ] && [ -s "$group_file" ]; then
                worker_pgid=$(cat "$group_file" 2>/dev/null || :)
                case "$worker_pgid" in
                  ''|*[!0-9]*) ;;
                  *) kill -"$signal" "-$worker_pgid" 2>/dev/null || : ;;
                esac
              fi
              if [ -n "$worker_pid" ]; then
                kill -"$signal" "$worker_pid" 2>/dev/null || :
              fi
              exit "$code"
            }
            pending_signal=""
            queue_signal() { pending_signal=$1; }
            trap cleanup_lane EXIT
            # Queue signals during worker launch so the PID and process-group
            # record are installed before cleanup starts.
            trap 'queue_signal HUP' HUP
            trap 'queue_signal INT' INT
            trap 'queue_signal TERM' TERM
            # Preserve the caller's input for noninteractive shells, which
            # otherwise give background jobs /dev/null on stdin.
            exec 9<&0
            if command -v setsid >/dev/null 2>&1 && [ -n "$group_file" ]; then
              # Disable job control so setsid does not fork: the background
              # PID is then also the new session/process-group ID.
              set +m
              setsid sh -c 'eval "$1"' sh "$command_text" <&9 &
              worker_pid=$!
              printf '%s\n' "$worker_pid" > "$group_file"
              # A private process group lets the parent hold the lane until
              # descendants exit, even if the command ends with exec.
            else
              # Keep the shell's normal background-job wait behavior where
              # setsid is unavailable; supported Fleet build hosts ship it.
              sh -c 'trap '\''rc=$?; trap - EXIT; wait || :; exit "$rc"'\'' EXIT; eval "$1"' sh "$command_text" <&9 &
              worker_pid=$!
            fi
            trap 'interrupt_lane HUP 129' HUP
            trap 'interrupt_lane INT 130' INT
            trap 'interrupt_lane TERM 143' TERM
            case "$pending_signal" in
              HUP) interrupt_lane HUP 129 ;;
              INT) interrupt_lane INT 130 ;;
              TERM) interrupt_lane TERM 143 ;;
            esac
            wait "$worker_pid"
            status=$?
            exit "$status"
          fi
          if [ -n "$lease_id" ]; then
            "$broker" checkin --lease "$lease_id" >/dev/null 2>&1 || :
          fi
        fi
      fi
      CARGO_TARGET_DIR="$HOME/build-cache/$repo"
      mkdir -p "$CARGO_TARGET_DIR"
      export CARGO_TARGET_DIR
      echo "fleet shell: broker unavailable; using shared fallback $CARGO_TARGET_DIR" >&2
    fi
  fi
fi
exec sh -c "$command_text"
"#;
    let script = script
        .replace("__PROFILE__", profile)
        .replace("__COMMAND__", &sh_quote_always(command));
    format!("sh -c {}", sh_quote_always(&script))
}

/// Run the command through one build-lock slot. Cargo commands also lease
/// their workspace target, including when Cargo is the first command.
fn under_build_lock(command: &str) -> String {
    if may_invoke_cargo(command) {
        format!("{BUILD_LOCK} {}", broker_aware_shell(command))
    } else {
        // The command input is shell text, so keep shell built-ins such as
        // exit, :, and ulimit inside a shell in every target mode.
        format!("{BUILD_LOCK} sh -c {}", sh_quote_always(command))
    }
}

/// A build on this host: under the build lock where this host has one
/// (the laptop does), and plainly, with a note, where it does not.
fn local_build_line(command: &str, cwd: Option<&str>) -> String {
    let change_dir = cwd
        .map(|dir| format!("cd {} || exit $?; ", sh_quote(dir)))
        .unwrap_or_default();
    let locked = under_build_lock(command);
    let fallback = if may_invoke_cargo(command) {
        broker_aware_shell(command)
    } else {
        format!("sh -c {}", sh_quote_always(command))
    };
    format!(
        "{change_dir}if [ -x {BUILD_LOCK} ]; then exec {locked}; fi; echo 'fleet shell: this host has no build lock; running without one' >&2; exec {fallback}"
    )
}

/// A codespace can use the same path when its fleet tools are installed.
/// If it lacks the lock, try the target broker directly and report the gap.
fn codespace_build_line(command: &str, cwd: Option<&str>) -> String {
    let change_dir = cwd
        .map(|dir| format!("cd {} || exit $?; ", sh_quote(dir)))
        .unwrap_or_default();
    let locked = under_build_lock(command);
    let fallback = if may_invoke_cargo(command) {
        broker_aware_shell(command)
    } else {
        format!("sh -c {}", sh_quote_always(command))
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
        Target::Actions(repo) => {
            if interactive {
                return Err(usage(
                    "the actions target runs a COMMAND on a GitHub Actions runner; \
                     it cannot open a shell",
                ));
            }
            if opts.build {
                return Err(usage(
                    "--build takes a host build-lock slot; the actions target \
                     dispatches through placement instead — drop --build",
                ));
            }
            if opts.cwd.is_some() {
                return Err(usage(
                    "--cwd does not apply to the actions target: the command runs \
                     in the runner's checkout of the repository",
                ));
            }
            // The plan is the dispatch step; run() drives it through
            // the session verbs instead of attaching to it.
            let args = argv(&[
                "queue",
                "dispatch",
                "--repo",
                repo,
                "--command",
                &command,
                "--dispatch",
                "--json",
            ]);
            ("fleet".to_string(), args)
        }
        Target::Codespace(name) => {
            if opts.detach {
                return Err(usage(
                    "--detach starts a laptop job; it needs --target laptop",
                ));
            }
            let mut args = argv(&["codespace", "ssh", "-c", name]);
            if !interactive {
                let line = if opts.build {
                    codespace_build_line(&command, opts.cwd.as_deref())
                } else {
                    match &opts.cwd {
                        Some(dir) => format!("cd {} && {command}", sh_quote(dir)),
                        None => command,
                    }
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

/// Whether placement has a GitHub Actions surface to dispatch to,
/// read from the surface registry the queue verbs print.
fn actions_reachable(world: &dyn World) -> bool {
    let out = world.fleet(&argv(&["queue", "surfaces"]));
    let Some(registry) = out.json() else {
        return false;
    };
    registry
        .get("surfaces")
        .and_then(Value::as_array)
        .is_some_and(|surfaces| {
            surfaces.iter().any(|s| {
                matches!(
                    s.get("kind").and_then(Value::as_str),
                    Some("github-actions") | Some("blacksmith")
                )
            })
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
            {"target": "actions", "reachable": actions_reachable(world),
             "how": "fleet queue dispatch (placement over the registered Actions surfaces)"},
            {"target": "local", "reachable": true, "how": "this machine"},
        ],
        "auto_would_pick": resolve(config, world, "auto", None, None).map(|(t, why)| json!({"target": t.id(), "because": why})).unwrap_or(Value::Null),
    })
}

/// How long to wait between session polls on the actions target.
pub const ACTIONS_POLL_SECS: u64 = 15;
/// The wait budget when --timeout does not say: an Actions run is a
/// build, so it gets the build budget, not the laptop command one.
pub const ACTIONS_DEFAULT_TIMEOUT_SECS: u64 = BUILD_TIMEOUT_SECS;

/// Drive one actions plan: dispatch through placement, then (unless
/// detached) poll the session to its terminal result. The plan's
/// program/args are the dispatch call itself.
fn run_actions(world: &dyn World, plan: &Plan, opts: &Options) -> R<Done> {
    let repo = match &plan.target {
        Target::Actions(repo) => repo.clone(),
        _ => return Ok(Done::Ran(world.attach(&plan.program, &plan.args))),
    };
    // Dispatch changes state on the far side (a session and a
    // workflow run), so it gets the patient call: cutting it short
    // would orphan the run.
    let dispatched = world.fleet_patient(&plan.args);
    if !dispatched.ok() {
        return Err(AgentError::Down(format!(
            "placement dispatch did not answer: {}",
            dispatched.why()
        )));
    }
    let record = dispatched
        .json()
        .ok_or_else(|| AgentError::Down("placement dispatch answered without JSON".to_string()))?;
    let handle = record
        .get("handle")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AgentError::Down("placement dispatch answered without a session handle".to_string())
        })?
        .to_string();
    if opts.detach {
        return Ok(Done::Printed(json!({
            "target": "actions",
            "repo": repo,
            "handle": handle,
            "state": record.get("state").cloned().unwrap_or(Value::Null),
            "run_url": record.get("run_url").cloned().unwrap_or(Value::Null),
            "because": plan.because,
            "note": format!("dispatched, not waited on; reconnect with: fleet queue session result {handle}"),
        })));
    }
    let budget = opts.timeout.unwrap_or(ACTIONS_DEFAULT_TIMEOUT_SECS);
    let deadline = world.now() + budget;
    loop {
        let out = world.fleet(&argv(&["queue", "session", "result", &handle, "--json"]));
        if !out.ok() {
            return Err(AgentError::Down(format!(
                "session result for {handle} did not answer: {}",
                out.why()
            )));
        }
        let result = out.json().ok_or_else(|| {
            AgentError::Down(format!("session result for {handle} answered without JSON"))
        })?;
        let terminal = result
            .get("terminal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if terminal {
            let state = result
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let code = if state == "succeeded" { 0 } else { 1 };
            return Ok(Done::PrintedCode(
                json!({
                    "target": "actions",
                    "repo": repo,
                    "handle": handle,
                    "state": state,
                    "run_url": result.get("run_url").cloned().unwrap_or(Value::Null),
                    "because": plan.because,
                }),
                code,
            ));
        }
        if world.now() >= deadline {
            let state = result
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(AgentError::Down(format!(
                "run {handle} still {state} after {budget}s; \
                 reconnect with: fleet queue session result {handle}"
            )));
        }
        std::thread::sleep(std::time::Duration::from_secs(ACTIONS_POLL_SECS));
    }
}

/// `fleet shell ...`.
pub fn run(config: &Config, world: &dyn World, args: &[String]) -> R<Done> {
    let a = parse(
        args,
        &["--build", "--detach", "--print"],
        &["--target", "--codespace", "--repo", "--cwd", "--timeout"],
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
    let (target, because) = resolve(
        config,
        world,
        &asked,
        a.last("--codespace"),
        a.last("--repo"),
    )?;
    let opts = Options {
        cwd: a.last("--cwd"),
        build: a.has("--build"),
        timeout: a.uint("--timeout")?,
        detach: a.has("--detach"),
    };
    let actions = matches!(target, Target::Actions(_));
    let plan = plan(target, because, &a.tail, &opts)?;
    if a.has("--print") {
        return Ok(Done::Printed(plan.to_json()));
    }
    if actions {
        return run_actions(world, &plan, &opts);
    }
    Ok(Done::Ran(world.attach(&plan.program, &plan.args)))
}
