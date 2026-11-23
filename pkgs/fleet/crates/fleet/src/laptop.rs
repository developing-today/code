//! Tier T0 (tiered/fleet) — developing-today/code#13717.
//!
//! The generic tiered command: the first option before any
//! fallback. It walks the fleet subcommand tiers T1 → T2 → T3 in
//! order, each attempt a literal invocation of that tier's own
//! implementation, advancing only when a tier fails to *serve*
//! (transport / auth / connect failure, or a tier-level timeout) —
//! never on a served result, whatever its exit code — and it
//! reports the tier that served.
//!
//! Tier 0 deliberately does not fall through to the wrapper (T7):
//! when tier 0 fails, the individual forms come into play as their
//! own literal commands — the subcommands again, then the raw
//! commands (T4–T6), then `laptop-ssh` — because an attempt under
//! tier 0's process shape is a different literal invocation than
//! the standalone command, and gate behaviour keys on the literal
//! shape.

use fleet_config::Config;

use crate::forms::{FORMS, Form, ServeOutcome, WALK_ORDER};

const EXIT_USAGE: i32 = 2;
const DEFAULT_TIMEOUT_S: u64 = 120;

const USAGE: &str = "\
Usage: fleet laptop <command> [args...]

Tier T0 (tiered/fleet) of the laptop route forms (#13717), and the
index of the whole matrix.

Commands:
  tiers    Print the codified form table (T0–T7) and the doctrine
  exec [--form T1|T2|T3] [--timeout S] [--cwd D] -- cmd...
           Run a command on the laptop. Without --form, walks
           T1 -> T2 -> T3 in order and reports the tier that
           served. --timeout is per attempt, in seconds, and
           always applies (default 120).";

fn tiers_text() -> String {
    let mut s = String::from(
        "The laptop route forms (#13717): tier 0, 3 methods x 2 shapes, the wrapper.\n\
         Circumstance decides the form; every form is tried as its own literal\n\
         command; every result names the form that served it.\n\n",
    );
    for f in FORMS {
        s.push_str(&format!(
            "{}  {:<10} {:<11} {}\n",
            f.id, f.method, f.shape, f.command
        ));
    }
    s.push_str(
        "\nComposition: tier 0 first. When it fails, the individual fleet\n\
         subcommands as their own commands, then the raw commands, then\n\
         the wrapper (T7, the fallback / simplified tier).\n",
    );
    s
}

fn form_by_id(id: &str) -> Option<Form> {
    FORMS.iter().find(|f| f.id == id).copied()
}

struct WalkArgs {
    form: Option<Form>,
    timeout_s: u64,
    cwd: Option<String>,
    cmd: Vec<String>,
}

fn parse_walk_args(args: &[String]) -> Result<WalkArgs, String> {
    let mut form = None;
    let mut timeout_s = DEFAULT_TIMEOUT_S;
    let mut cwd = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            let cmd: Vec<String> = args[i + 1..].to_vec();
            if cmd.is_empty() {
                return Err("exec: no command after --".into());
            }
            return Ok(WalkArgs {
                form,
                timeout_s,
                cwd,
                cmd,
            });
        } else if a == "--form" {
            i += 1;
            let v = args.get(i).ok_or("exec: --form needs a value")?;
            form = Some(form_by_id(v).ok_or_else(|| format!("exec: unknown form {v}"))?);
        } else if let Some(v) = a.strip_prefix("--form=") {
            form = Some(form_by_id(v).ok_or_else(|| format!("exec: unknown form {v}"))?);
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
        } else if a == "--cwd" {
            i += 1;
            cwd = Some(
                args.get(i)
                    .ok_or("exec: --cwd needs a value")?
                    .to_string(),
            );
        } else if let Some(v) = a.strip_prefix("--cwd=") {
            cwd = Some(v.to_string());
        } else {
            return Err(format!("exec: unexpected argument {a} (command goes after --)"));
        }
        i += 1;
    }
    Err("exec: missing -- before the command".into())
}

fn attempt(config: &Config, form: Form, cwd: Option<&str>, timeout_s: u64, cmd: &[String]) -> ServeOutcome {
    match form.id {
        "T1" => crate::commander::serve(config, cwd, timeout_s, cmd),
        "T2" => crate::http::serve(config, cwd, timeout_s, cmd),
        "T3" => match crate::ssh::serve(config, cwd, timeout_s, cmd) {
            // ssh's own failure exit is 255 (connect/auth/protocol):
            // by ssh convention that is the tier failing to serve,
            // not the command's result. Recorded casualty: a remote
            // command that itself exits 255 over ssh reads as a
            // tier failure here.
            ServeOutcome::Served(255) => {
                ServeOutcome::Failed("ssh exited 255 (connect/auth failure)".into())
            }
            other => other,
        },
        _ => ServeOutcome::Failed(format!("form {} is not a fleet subcommand tier", form.id)),
    }
}

fn walk(config: &Config, args: &[String]) -> i32 {
    let parsed = match parse_walk_args(args) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("laptop: {m}\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let order: Vec<Form> = match parsed.form {
        Some(f) => {
            if !WALK_ORDER.contains(&f) {
                eprintln!(
                    "laptop: --form {} cannot be attempted by tier 0 (raw forms and the \
                     wrapper are run as their own literal commands)",
                    f.id
                );
                return EXIT_USAGE;
            }
            vec![f]
        }
        None => WALK_ORDER.to_vec(),
    };
    for form in order {
        eprintln!("fleet: trying form {}", form.label());
        match attempt(config, form, parsed.cwd.as_deref(), parsed.timeout_s, &parsed.cmd) {
            ServeOutcome::Served(code) => {
                eprintln!("fleet: served by form {}", form.label());
                return code;
            }
            ServeOutcome::TimedOut => {
                eprintln!(
                    "fleet: form {} timed out after {}s; trying next",
                    form.label(),
                    parsed.timeout_s
                );
            }
            ServeOutcome::Failed(m) => {
                eprintln!("fleet: form {} failed: {m}; trying next", form.label());
            }
        }
    }
    eprintln!("fleet: no tier served the call (the individual forms, then the raw forms, then laptop-ssh remain)");
    1
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            0
        }
        Some("tiers") | Some("forms") => {
            print!("{}", tiers_text());
            0
        }
        Some("exec") => walk(config, &args[1..]),
        _ => {
            eprintln!("{USAGE}");
            EXIT_USAGE
        }
    }
}
