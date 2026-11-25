//! The `FLEET_CTX_*` env contract — fleet context propagation build
//! 2/5 (developing-today/code#14188, decision record for #10357).
//!
//! Every fleet action carries its origin: a small fixed set of
//! environment variables, captured once at the outermost point
//! (init sourcing, launchers, hooks — builds 1/5 and 3/5) and passed
//! unchanged through every carrier this module renders:
//!
//! | carrier                    | encoding                                   |
//! |----------------------------|--------------------------------------------|
//! | process env                | `FLEET_CTX_<NAME>=<value>` (canonical, Q1) |
//! | laptop-agent exec / jobs   | the request spec's `env` object            |
//! | SSH                        | explicit `export …;` prefix on the line     |
//! | queue / secrets broker     | `X-Fleet-Ctx-<Name>` request headers        |
//! | file transfer              | the transfer's exec spec `env` + trip log   |
//!
//! Hard constraints (#10357, all four honored here):
//! - zero model effort: [`Ctx::from_env`] reads the process env; no
//!   caller passes anything by hand;
//! - cheap on the hot path: a fixed-table scan of the env, no I/O, no
//!   round trip;
//! - no secrets: values are ids and hashes only, restricted to a
//!   header- and shell-safe charset and a length cap — a value outside
//!   the grammar is dropped, never escaped and forwarded;
//! - absence is representable: every variable is optional; an empty
//!   [`Ctx`] renders no header, no export and no log field, and no
//!   carrier errors on it.
//!
//! Q1 default (decision record §3): the env-carried record is
//! canonical. Spool files and trip logs are per-surface caches keyed by
//! `FLEET_CTX_ID`; each surface owns its own sweep.
//!
//! Unknown `FLEET_CTX_*` names do not travel: the set is fixed so a
//! carrier can never be used to smuggle arbitrary environment.

/// One variable of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxVar {
    /// The environment name, e.g. `FLEET_CTX_TASK_ID`.
    pub env: &'static str,
    /// The HTTP header carrier, e.g. `X-Fleet-Ctx-Task-Id`.
    pub header: &'static str,
    /// Who sets it (documentation; rendered by `fleet ctx vars`).
    pub producer: &'static str,
    /// What it holds.
    pub meaning: &'static str,
}

/// The env prefix every contract variable shares.
pub const PREFIX: &str = "FLEET_CTX_";
/// The header prefix every contract variable shares.
pub const HEADER_PREFIX: &str = "X-Fleet-Ctx-";
/// Longest value that travels (bytes). Ids and hashes fit with room.
pub const MAX_VALUE_LEN: usize = 256;

/// The fixed variable set, in canonical (log and render) order.
/// `FLEET_CTX_ID` is first: it is the join key every store logs.
pub const VARS: &[CtxVar] = &[
    CtxVar {
        env: "FLEET_CTX_ID",
        header: "X-Fleet-Ctx-Id",
        producer: "init sourcing (fleet-ctx-init.sh) / launcher",
        meaning: "join key across stores: task-<id> when queue-launched, else origin-host-pid",
    },
    CtxVar {
        env: "FLEET_CTX_ORIGIN",
        header: "X-Fleet-Ctx-Origin",
        producer: "init sourcing",
        meaning: "laptop user-context: drew-interactive|opencode|claude|systemd-unit[:inner]|unknown",
    },
    CtxVar {
        env: "FLEET_CTX_HOST",
        header: "X-Fleet-Ctx-Host",
        producer: "init sourcing",
        meaning: "hostname at capture",
    },
    CtxVar {
        env: "FLEET_CTX_USER",
        header: "X-Fleet-Ctx-User",
        producer: "init sourcing",
        meaning: "acting OS user / user-context at capture",
    },
    CtxVar {
        env: "FLEET_CTX_PID",
        header: "X-Fleet-Ctx-Pid",
        producer: "init sourcing",
        meaning: "pid of the capturing shell",
    },
    CtxVar {
        env: "FLEET_CTX_ANCESTRY",
        header: "X-Fleet-Ctx-Ancestry",
        producer: "init sourcing / session hooks",
        meaning: "compact ancestor summary, '/'-joined, e.g. cron/muse/opencode",
    },
    CtxVar {
        env: "FLEET_CTX_TASK_ID",
        header: "X-Fleet-Ctx-Task-Id",
        producer: "queue worker / launcher",
        meaning: "fleet queue task id when launched from the queue",
    },
    CtxVar {
        env: "FLEET_CTX_TASK_NAME",
        header: "X-Fleet-Ctx-Task-Name",
        producer: "queue worker / launcher",
        meaning: "queue task name (slug form; titles with spaces are dropped)",
    },
    CtxVar {
        env: "FLEET_CTX_MUSE_SESSION",
        header: "X-Fleet-Ctx-Muse-Session",
        producer: "Muse launcher",
        meaning: "Muse session id",
    },
    CtxVar {
        env: "FLEET_CTX_MODEL",
        header: "X-Fleet-Ctx-Model",
        producer: "session hooks",
        meaning: "internal model name serving the session",
    },
    CtxVar {
        env: "FLEET_CTX_OPENCODE_SESSION",
        header: "X-Fleet-Ctx-Opencode-Session",
        producer: "OpenCode session hook",
        meaning: "OpenCode session id",
    },
    CtxVar {
        env: "FLEET_CTX_OPENCODE_PARENT",
        header: "X-Fleet-Ctx-Opencode-Parent",
        producer: "OpenCode session hook",
        meaning: "OpenCode parent session id",
    },
    CtxVar {
        env: "FLEET_CTX_SCHEDULED_JOB",
        header: "X-Fleet-Ctx-Scheduled-Job",
        producer: "OpenChamber scheduler",
        meaning: "scheduled job that originated the session",
    },
    CtxVar {
        env: "FLEET_CTX_MESSAGE_ID",
        header: "X-Fleet-Ctx-Message-Id",
        producer: "session hooks",
        meaning: "triggering message id, where the surface exposes one",
    },
    CtxVar {
        env: "FLEET_CTX_GIT_HEADS",
        header: "X-Fleet-Ctx-Git-Heads",
        producer: "init sourcing",
        meaning: "repo=sha pairs, ','-joined (hatch, hatch-personal, working repo)",
    },
    CtxVar {
        env: "FLEET_CTX_INVOCATION",
        header: "X-Fleet-Ctx-Invocation",
        producer: "init sourcing under systemd (Q3 default)",
        meaning: "systemd unit invocation id",
    },
];

/// Look a contract variable up by its env name.
pub fn var_by_env(name: &str) -> Option<&'static CtxVar> {
    VARS.iter().find(|v| v.env == name)
}

/// Look a contract variable up by its header name (case-insensitive,
/// as HTTP header names are).
pub fn var_by_header(name: &str) -> Option<&'static CtxVar> {
    VARS.iter().find(|v| v.header.eq_ignore_ascii_case(name))
}

/// Whether `value` is inside the value grammar: 1..=MAX_VALUE_LEN
/// bytes of `[A-Za-z0-9._:@/+=,~-]`. The charset is safe unquoted in
/// an HTTP header, a POSIX shell word and a log line; anything else
/// (spaces, quotes, control bytes, `$`) is refused.
pub fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_VALUE_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:@/+=,~-".contains(&b))
}

/// A (possibly partial, possibly empty) context record: contract
/// variables with grammar-valid values, in canonical order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ctx {
    pairs: Vec<(&'static CtxVar, String)>,
}

impl Ctx {
    /// Build from `(env name, value)` pairs. Unknown names and
    /// out-of-grammar values are dropped; the first occurrence of a
    /// name wins.
    pub fn from_pairs<I, K, V>(pairs: I) -> Ctx
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut found: Vec<Option<String>> = vec![None; VARS.len()];
        for (k, v) in pairs {
            if let Some(i) = VARS.iter().position(|var| var.env == k.as_ref()) {
                let v = v.as_ref();
                if found[i].is_none() && valid_value(v) {
                    found[i] = Some(v.to_string());
                }
            }
        }
        Ctx::collect(found)
    }

    /// Capture from the current process environment (the canonical
    /// record). Non-UTF-8 values are absent, not errors.
    pub fn from_env() -> Ctx {
        let found: Vec<Option<String>> = VARS
            .iter()
            .map(|var| std::env::var(var.env).ok().filter(|v| valid_value(v)))
            .collect();
        Ctx::collect(found)
    }

    /// Decode from HTTP headers (`X-Fleet-Ctx-*`, case-insensitive
    /// names). Other headers are ignored.
    pub fn from_headers<I, K, V>(headers: I) -> Ctx
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut found: Vec<Option<String>> = vec![None; VARS.len()];
        for (k, v) in headers {
            if let Some(i) = VARS
                .iter()
                .position(|var| var.header.eq_ignore_ascii_case(k.as_ref()))
            {
                let v = v.as_ref().trim();
                if found[i].is_none() && valid_value(v) {
                    found[i] = Some(v.to_string());
                }
            }
        }
        Ctx::collect(found)
    }

    fn collect(found: Vec<Option<String>>) -> Ctx {
        Ctx {
            pairs: VARS
                .iter()
                .zip(found)
                .filter_map(|(var, v)| v.map(|v| (var, v)))
                .collect(),
        }
    }

    /// No variable present (an interactive shell or pre-contract
    /// caller): every carrier renders nothing.
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The value of one variable by env name.
    pub fn get(&self, env: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(var, _)| var.env == env)
            .map(|(_, v)| v.as_str())
    }

    /// The join key (`FLEET_CTX_ID`), when present.
    pub fn id(&self) -> Option<&str> {
        self.get("FLEET_CTX_ID")
    }

    /// `(env name, value)` pairs in canonical order — the exec/jobs
    /// spec `env` carrier.
    pub fn env_pairs(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.pairs.iter().map(|(var, v)| (var.env, v.as_str()))
    }

    /// `(header name, value)` pairs in canonical order.
    pub fn header_pairs(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.pairs.iter().map(|(var, v)| (var.header, v.as_str()))
    }

    /// Raw HTTP/1.1 header lines (`Name: value\r\n` each) for the
    /// hand-written request heads the brokers' clients send. Empty
    /// when the record is empty.
    pub fn header_lines(&self) -> String {
        self.header_pairs()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect()
    }

    /// The SSH carrier: an explicit `export K=V …; ` prefix for the
    /// remote command line (works whatever the remote sshd's
    /// `AcceptEnv` says). Values are grammar-valid, so they are safe
    /// unquoted; they are single-quoted anyway for defence in depth.
    /// Empty when the record is empty.
    pub fn shell_prefix(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let words: Vec<String> = self
            .env_pairs()
            .map(|(k, v)| format!("{k}='{v}'"))
            .collect();
        format!("export {}; ", words.join(" "))
    }

    /// The session-proxy frame carrier (`ctx` on a proxy request
    /// frame): env name -> value, or `None` when the record is empty
    /// so the frame field is omitted entirely.
    pub fn to_frame(&self) -> Option<std::collections::BTreeMap<String, String>> {
        if self.is_empty() {
            return None;
        }
        Some(
            self.env_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// The per-trip log rendering: `FLEET_CTX_ID=<v> FLEET_CTX_…=<v>`
    /// in canonical order. Every leg logs this same rendering, so one
    /// grep for `FLEET_CTX_ID=<id>` follows a context across logs.
    pub fn log_fields(&self) -> String {
        self.env_pairs()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_fixed_prefixed_and_header_names_line_up() {
        assert!(VARS.len() <= 16, "the set stays small and fixed");
        assert_eq!(VARS[0].env, "FLEET_CTX_ID", "join key renders first");
        for var in VARS {
            assert!(var.env.starts_with(PREFIX), "{}", var.env);
            assert!(var.header.starts_with(HEADER_PREFIX), "{}", var.header);
            // Header name is the env suffix in Title-Case-With-Dashes.
            let suffix = &var.env[PREFIX.len()..];
            let want: String = suffix
                .split('_')
                .map(|w| {
                    let mut c = w.chars();
                    let first = c.next().unwrap().to_ascii_uppercase();
                    std::iter::once(first)
                        .chain(c.map(|c| c.to_ascii_lowercase()))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("-");
            assert_eq!(var.header, format!("{HEADER_PREFIX}{want}"));
            assert_eq!(var_by_env(var.env), Some(var));
            assert_eq!(var_by_header(&var.header.to_lowercase()), Some(var));
        }
    }

    #[test]
    fn grammar_refuses_shell_and_header_metacharacters() {
        for ok in ["task-22655", "a.b:c@d/e+f=g,h~i_j", "0123456789abcdef"] {
            assert!(valid_value(ok), "{ok}");
        }
        for bad in [
            "",
            "two words",
            "q'uote",
            "d\"q",
            "$HOME",
            "a;b",
            "a\r\nX-Evil: 1",
            "a>b",
            "`x`",
        ] {
            assert!(!valid_value(bad), "{bad:?}");
        }
        assert!(valid_value(&"x".repeat(MAX_VALUE_LEN)));
        assert!(!valid_value(&"x".repeat(MAX_VALUE_LEN + 1)));
    }

    #[test]
    fn absent_record_renders_nothing_on_every_carrier() {
        let ctx = Ctx::from_pairs(Vec::<(String, String)>::new());
        assert!(ctx.is_empty());
        assert_eq!(ctx.header_lines(), "");
        assert_eq!(ctx.shell_prefix(), "");
        assert_eq!(ctx.log_fields(), "");
        assert_eq!(ctx.id(), None);
    }

    #[test]
    fn unknown_names_and_bad_values_do_not_travel_partial_record_does() {
        let ctx = Ctx::from_pairs([
            ("FLEET_CTX_TASK_ID", "22655"),
            ("FLEET_CTX_SECRET_TOKEN", "abc"),
            ("FLEET_CTX_ORIGIN", "has space"),
            ("PATH", "/usr/bin"),
        ]);
        assert_eq!(ctx.log_fields(), "FLEET_CTX_TASK_ID=22655");
        assert_eq!(ctx.id(), None, "partial record: no id is fine");
    }

    #[test]
    fn canonical_order_and_first_occurrence_win() {
        let ctx = Ctx::from_pairs([
            ("FLEET_CTX_TASK_ID", "1"),
            ("FLEET_CTX_ID", "task-1"),
            ("FLEET_CTX_ID", "task-2"),
        ]);
        assert_eq!(ctx.log_fields(), "FLEET_CTX_ID=task-1 FLEET_CTX_TASK_ID=1");
    }

    #[test]
    fn header_round_trip_is_byte_identical() {
        let ctx = Ctx::from_pairs([
            ("FLEET_CTX_ID", "task-22655"),
            ("FLEET_CTX_ORIGIN", "systemd-unit:opencode"),
            ("FLEET_CTX_GIT_HEADS", "hatch=0a1b,work=9f8e"),
        ]);
        let lines = ctx.header_lines();
        assert_eq!(
            lines,
            "X-Fleet-Ctx-Id: task-22655\r\nX-Fleet-Ctx-Origin: systemd-unit:opencode\r\nX-Fleet-Ctx-Git-Heads: hatch=0a1b,work=9f8e\r\n"
        );
        // Parse the lines back the way a server would (lower-cased
        // names, as some HTTP stacks normalize them).
        let parsed = Ctx::from_headers(lines.split("\r\n").filter(|l| !l.is_empty()).map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.to_ascii_lowercase(), v.to_string())
        }));
        assert_eq!(parsed, ctx);
        assert_eq!(parsed.log_fields(), ctx.log_fields());
    }

    #[test]
    fn header_injection_is_impossible_by_grammar() {
        let ctx = Ctx::from_pairs([("FLEET_CTX_ID", "x\r\nX-Agent-Token: stolen")]);
        assert!(ctx.is_empty());
        let from_hdr = Ctx::from_headers([("X-Fleet-Ctx-Id", "ok"), ("X-Agent-Token", "t")]);
        assert_eq!(from_hdr.log_fields(), "FLEET_CTX_ID=ok");
    }

    #[test]
    fn shell_prefix_exports_through_a_real_shell_unchanged() {
        let ctx = Ctx::from_pairs([
            ("FLEET_CTX_ID", "task-22655"),
            ("FLEET_CTX_ANCESTRY", "cron/muse/opencode"),
        ]);
        let line = format!(
            "{}cd / && printf '%s|%s' \"$FLEET_CTX_ID\" \"$FLEET_CTX_ANCESTRY\"",
            ctx.shell_prefix()
        );
        // The SSH leg hands this line to the remote login shell; run it
        // under a scrubbed sh exactly as sshd would (no inherited env).
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&line)
            .env_clear()
            .output()
            .expect("sh runs");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "task-22655|cron/muse/opencode"
        );
    }
}
