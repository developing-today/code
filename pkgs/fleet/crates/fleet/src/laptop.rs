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
//! Selection goes through `fleet-pool` (#14722, design #13752
//! §3.5 step 1 — the walker is the primitive's first adopter):
//! the tiers are pool members under one host group, the pool's
//! selection picks each attempt, and each attempt's outcome is
//! classified back into the pool (`Served` is a success; `Failed`
//! and `TimedOut` are serve-failures, whose classification always
//! advances). The walker's own ladder code is gone; the only
//! walker-specific piece left is the attempted-set strategy
//! below, the same shape the crate's walker oracle tests use
//! (`crates/fleet-pool/tests/walker.rs`): a form already attempted
//! in this walk is not re-selected, because a single walk tries
//! each tier at most once even though a first serve-failure only
//! demotes the member in the pool's longer-lived state machine.
//!
//! Tier 0 deliberately does not fall through to the wrapper (T7):
//! when tier 0 fails, the individual forms come into play as their
//! own literal commands — the subcommands again, then the raw
//! commands (T4–T6), then `laptop-ssh` — because an attempt under
//! tier 0's process shape is a different literal invocation than
//! the standalone command, and gate behaviour keys on the literal
//! shape.

use std::collections::HashSet;

use fleet_config::Config;
use fleet_pool::{
    AttemptKind, Candidate, Classification, NodeId, Pool, ServedDisposition, Strategy,
};

use crate::forms::{Form, ServeOutcome, FORMS, T1, T2, T3};

/// The fleet subcommand tiers tier 0 walks, in declaration order:
/// the members of the walk's pool. The wrapper (T7) is
/// deliberately NOT a member: it is the agent's own final step
/// after tier 0 and the individual forms, never something tier 0
/// papers over.
const WALK_FORMS: [Form; 3] = [T1, T2, T3];

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
            cwd = Some(args.get(i).ok_or("exec: --cwd needs a value")?.to_string());
        } else if let Some(v) = a.strip_prefix("--cwd=") {
            cwd = Some(v.to_string());
        } else {
            return Err(format!(
                "exec: unexpected argument {a} (command goes after --)"
            ));
        }
        i += 1;
    }
    Err("exec: missing -- before the command".into())
}

fn attempt(
    config: &Config,
    form: Form,
    cwd: Option<&str>,
    timeout_s: u64,
    cmd: &[String],
) -> ServeOutcome {
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

/// The walker's cursor over the pool: declaration order over the
/// forms it has not attempted yet this walk. The pool supplies
/// availability (a walled form never becomes a candidate), state,
/// advance verdicts, and the decision log; the cursor is the
/// walker's own, exactly as in the `fleet-pool` walker oracle
/// tests (crates/fleet-pool/tests/walker.rs).
#[derive(Default)]
struct WalkOrder {
    attempted: HashSet<NodeId>,
}

impl Strategy for WalkOrder {
    fn name(&self) -> &str {
        "walker-ordered"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        candidates
            .iter()
            .position(|c| !self.attempted.contains(&c.id))
    }
}

/// Classify a form that did not serve: a serve-failure, which the
/// pool always advances past.
fn serve_failure(member: &NodeId, evidence: String) -> Classification {
    Classification {
        kind: AttemptKind::ServeFailure,
        served_disposition: ServedDisposition::Hold,
        advance: true,
        evidence_scope: member.clone(),
        reset_hint: None,
        evidence,
    }
}

/// One tier-0 walk over `forms`, selecting through `fleet-pool`.
/// `attempt_fn` runs one form and reports how it ended. Returns
/// the serving form and the served exit code, or `None` when no
/// form served. Oracle: #13717 — advance only on a failure to
/// serve, never on a served result; every result names the
/// serving form (the `Served` arm's form, also the last selection
/// in the pool's decision log).
fn walk_forms(
    forms: &[Form],
    timeout_s: u64,
    mut attempt_fn: impl FnMut(Form) -> ServeOutcome,
) -> Option<(Form, i32)> {
    let mut pool = Pool::new();
    let host = NodeId::new("host");
    pool.add_group(host.clone(), None);
    for form in forms {
        pool.add_member(NodeId::new(form.id), Some(&host));
    }
    let mut order = WalkOrder::default();
    // The pool is per-walk and time is its logical clock; a single
    // walk happens at one logical instant.
    let now = 0;
    loop {
        let Some(member) = pool.select(&mut order, &|_| None, now) else {
            return None;
        };
        order.attempted.insert(member.clone());
        let form = forms
            .iter()
            .find(|f| f.id == member.as_str())
            .copied()
            .expect("the pool only selects members the walker added");
        eprintln!("fleet: trying form {}", form.label());
        match attempt_fn(form) {
            ServeOutcome::Served(code) => {
                pool.record_success(&member, now);
                eprintln!("fleet: served by form {}", form.label());
                return Some((form, code));
            }
            ServeOutcome::TimedOut => {
                let fx = pool.record_outcome(
                    &member,
                    &serve_failure(&member, "timed out".to_string()),
                    now,
                );
                debug_assert!(fx.advance, "a failure to serve always advances");
                eprintln!(
                    "fleet: form {} timed out after {}s; trying next",
                    form.label(),
                    timeout_s
                );
            }
            ServeOutcome::Failed(m) => {
                let fx = pool.record_outcome(&member, &serve_failure(&member, m.clone()), now);
                debug_assert!(fx.advance, "a failure to serve always advances");
                eprintln!("fleet: form {} failed: {m}; trying next", form.label());
            }
        }
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
    let forms: Vec<Form> = match parsed.form {
        Some(f) => {
            if !WALK_FORMS.contains(&f) {
                eprintln!(
                    "laptop: --form {} cannot be attempted by tier 0 (raw forms and the \
                     wrapper are run as their own literal commands)",
                    f.id
                );
                return EXIT_USAGE;
            }
            vec![f]
        }
        None => WALK_FORMS.to_vec(),
    };
    match walk_forms(&forms, parsed.timeout_s, |form| {
        attempt(
            config,
            form,
            parsed.cwd.as_deref(),
            parsed.timeout_s,
            &parsed.cmd,
        )
    }) {
        Some((_form, code)) => code,
        None => {
            eprintln!(
                "fleet: no tier served the call (the individual forms, then the raw forms, then laptop-ssh remain)"
            );
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::{T1, T2, T3};
    use std::cell::RefCell;

    /// Run one scripted walk over T1–T3 and return the outcome
    /// plus the forms attempted, in order.
    fn scripted_walk(script: impl Fn(Form) -> ServeOutcome) -> (Option<(Form, i32)>, Vec<Form>) {
        let attempted = RefCell::new(Vec::new());
        let out = walk_forms(&WALK_FORMS, 120, |form| {
            attempted.borrow_mut().push(form);
            script(form)
        });
        (out, attempted.into_inner())
    }

    // Oracle: #13717 — the first form serves on a healthy estate
    // and the result names it; no other form is attempted.
    #[test]
    fn walker_serves_via_first_form_and_names_it() {
        let (out, attempted) = scripted_walk(|_| ServeOutcome::Served(0));
        assert_eq!(out, Some((T1, 0)));
        assert_eq!(attempted, vec![T1]);
    }

    // Oracle: #13717 — the walk advances only when a form fails
    // to serve (transport failure or a tier-level timeout).
    #[test]
    fn walker_advances_only_on_failure_to_serve() {
        let (out, attempted) = scripted_walk(|form| match form.id {
            "T1" => ServeOutcome::Failed("connect".into()),
            "T2" => ServeOutcome::TimedOut,
            _ => ServeOutcome::Served(7),
        });
        assert_eq!(out, Some((T3, 7)));
        assert_eq!(attempted, vec![T1, T2, T3]);
    }

    // Oracle: #13717 — a served result is never routed around,
    // whatever its exit code.
    #[test]
    fn walker_never_advances_on_a_served_result() {
        let (out, attempted) = scripted_walk(|_| ServeOutcome::Served(42));
        assert_eq!(out, Some((T1, 42)));
        assert_eq!(attempted, vec![T1]);
    }

    // Oracle: #13717 — when no form serves, the walk exhausts
    // every form exactly once and reports no server.
    #[test]
    fn walker_exhausts_when_no_form_serves() {
        let (out, attempted) = scripted_walk(|_| ServeOutcome::Failed("down".into()));
        assert_eq!(out, None);
        assert_eq!(attempted, vec![T1, T2, T3]);
    }
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
