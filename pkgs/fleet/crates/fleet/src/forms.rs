//! The laptop route forms (developing-today/code#13717): the shared
//! vocabulary of the codified matrix — tier 0 (the generic tiered
//! command), 3 methods × 2 shapes (T1–T6), and the `laptop-ssh`
//! wrapper (T7) — and the outcome type every fleet-shape form's exec
//! returns, so tier 0 can compose the subcommand tiers without any
//! of them knowing about the others.
//!
//! Why forms, not a ladder: the platform's execution-risk behaviour
//! keys on the literal command shape an agent runs, so each form is
//! a first-class, separately invocable command, and every result
//! names the exact form that served it (method × shape). Tier 0's
//! internal attempts run under tier 0's own process shape, which is
//! itself a different literal invocation than the standalone
//! subcommands — after tier 0 fails, the individual forms are tried
//! as their own commands, then the raw forms, then the wrapper.

use std::process::Command;
use std::time::{Duration, Instant};

/// How one form's exec attempt ended.
pub enum ServeOutcome {
    /// The form served the call; the value is the remote command's
    /// exit code (its output has already been printed).
    Served(i32),
    /// The form's own deadline expired before the command completed
    /// (the form cleaned up after itself: the commander form
    /// force-terminates, the http form cancels through the daemon).
    /// Tier 0 treats this as "this form did not serve" and moves to
    /// the next form; the standalone form maps it to exit 124.
    TimedOut,
    /// The form could not serve at all (transport, auth, connect,
    /// missing local piece). The string says why; it never carries a
    /// secret value.
    Failed(String),
}

/// One codified form of reaching the laptop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    pub id: &'static str,
    pub method: &'static str,
    pub shape: &'static str,
    /// The literal command, as documented for agents to run as-is.
    pub command: &'static str,
}

pub const T0: Form = Form {
    id: "T0",
    method: "tiered",
    shape: "fleet",
    command: "fleet laptop exec [--timeout S] -- cmd...",
};
pub const T1: Form = Form {
    id: "T1",
    method: "commander",
    shape: "fleet",
    command: "fleet commander exec [--cwd D] --timeout S -- cmd...",
};
pub const T2: Form = Form {
    id: "T2",
    method: "http",
    shape: "fleet",
    command: "fleet http exec [--cwd D] --timeout S -- cmd...",
};
pub const T3: Form = Form {
    id: "T3",
    method: "ssh",
    shape: "fleet",
    command: "fleet ssh exec --timeout S -- cmd...",
};
pub const T4: Form = Form {
    id: "T4",
    method: "commander",
    shape: "raw",
    command: "curl to https://commander.security.cab/mcp (recipe in TOOLS.d/laptop.md)",
};
pub const T5: Form = Form {
    id: "T5",
    method: "http",
    shape: "raw",
    command: "curl to the daemon API via the session proxy (recipe in TOOLS.d/laptop.md)",
};
pub const T6: Form = Form {
    id: "T6",
    method: "ssh",
    shape: "raw",
    command: "ssh (literal invocation in TOOLS.d/laptop.md) user@100.99.113.103 'cmd...'",
};
pub const T7: Form = Form {
    id: "T7",
    method: "wrapper",
    shape: "laptop-ssh",
    command: "laptop-ssh 'cmd...'",
};

/// All eight codified forms, tier 0 first.
pub const FORMS: [Form; 8] = [T0, T1, T2, T3, T4, T5, T6, T7];

/// The fleet subcommand tiers tier 0 walks, in order. The wrapper
/// (T7) is deliberately NOT in this list: it is the agent's own
/// final step after tier 0 and the individual forms, never
/// something tier 0 papers over.
pub const WALK_ORDER: [Form; 3] = [T1, T2, T3];

impl Form {
    /// The label every result and log line names: `T2 http/fleet`.
    pub fn label(self) -> String {
        format!("{} {}/{}", self.id, self.method, self.shape)
    }
}

/// Announce the form about to serve an exec (stderr only: stdout
/// stays the remote command's own).
pub fn announce(form: Form) {
    eprintln!("fleet: form {}", form.label());
}

/// Single-quote a string for a POSIX shell command line.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Spawn a child (stdio inherited) and bound the wait by a deadline:
/// on expiry the child is killed and the outcome is `TimedOut`
/// (callers map it to exit 124). Used by the ssh form, whose child
/// is a local process; the commander and http forms manage their
/// remote processes through their own protocols instead.
pub fn spawn_with_deadline(cmd: &mut Command, timeout_s: u64) -> ServeOutcome {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ServeOutcome::Failed(format!("spawn: {e}")),
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_s.max(1));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ServeOutcome::Served(status.code().unwrap_or(1)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return ServeOutcome::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return ServeOutcome::Failed(format!("wait: {e}")),
        }
    }
}
