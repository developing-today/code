//! Reason-required closes and closure records for `fleet gh`
//! (developing-today/code#9221).
//!
//! `issue close`, `pr close` and their `reopen` counterparts pass
//! through the rotation like every other gh verb, except that a close
//! must carry a `--reason` from the taxonomy. The reason is written
//! into the close comment, and every successful close or reopen
//! appends one closure record (JSON line) to the closure log, which
//! lives in the fleet state directory so it is backed up with it.
//! `fleet gh closures [<item>]` reads the log back.

use fleet_config::Config;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const POLICY_ERROR: &str = "Please give a reason. This is required by policy.";
pub const REASONS: [&str; 4] = [
    "implemented",
    "superseded",
    "cancelled",
    "started-incorrectly",
];
pub const RECORD_SCHEMA: &str = "fleet-closure/v1";
const LOG_FILE: &str = "closure-records.jsonl";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub action: &'static str,
    pub kind: &'static str,
    pub item: String,
    pub reason: Option<String>,
    pub reason_text: Option<String>,
    pub successor: Option<String>,
    pub repo: Option<String>,
    pub gh_args: Vec<String>,
}

/// Recognise `issue|pr close|reopen`. Returns Ok(None) for every
/// other gh call, which then passes through untouched.
pub fn parse(args: &[String]) -> Result<Option<Intent>, String> {
    if args.len() < 3 {
        return Ok(None);
    }
    let kind = match args[0].as_str() {
        "issue" => "issue",
        "pr" => "pr",
        _ => return Ok(None),
    };
    let action = match args[1].as_str() {
        "close" => "close",
        "reopen" => "reopen",
        _ => return Ok(None),
    };
    let mut item: Option<String> = None;
    let mut reason = None;
    let mut reason_text = None;
    let mut successor = None;
    let mut repo = None;
    let mut gh_args = vec![args[0].clone(), args[1].clone()];
    let mut i = 2;
    while i < args.len() {
        let a = args[i].as_str();
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if a.starts_with("--") => (n, Some(v.to_string())),
            _ => (a, None),
        };
        let takes_value = matches!(
            name,
            "--reason"
                | "-r"
                | "--reason-text"
                | "--successor"
                | "--repo"
                | "-R"
                | "--comment"
                | "-c"
        );
        if takes_value {
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    match args.get(i) {
                        Some(v) => v.clone(),
                        None => return Err(format!("flag '{name}' needs a value")),
                    }
                }
            };
            match name {
                "--reason" | "-r" => reason = Some(value),
                "--reason-text" => reason_text = Some(value),
                "--successor" => successor = Some(value),
                "--repo" | "-R" => {
                    repo = Some(value.clone());
                    gh_args.push(name.to_string());
                    gh_args.push(value);
                }
                _ => {
                    return Err(
                        "--comment is not accepted on a closure: use --reason-text".to_string()
                    );
                }
            }
        } else if item.is_none() && !a.starts_with('-') {
            item = Some(a.to_string());
            gh_args.push(a.to_string());
        } else {
            gh_args.push(args[i].clone());
        }
        i += 1;
    }
    let item = match item {
        Some(raw) => item_number(&raw)?,
        None => return Err(format!("{} {} needs an item number or URL", kind, action)),
    };
    if action == "close" {
        let r = match reason.as_deref() {
            None => return Err(POLICY_ERROR.to_string()),
            Some(r) => r,
        };
        if !REASONS.contains(&r) {
            return Err(format!(
                "unknown reason '{r}': use one of {}",
                REASONS.join(", ")
            ));
        }
        if r == "superseded" && successor.as_deref().is_none_or(str::is_empty) {
            return Err("reason 'superseded' needs --successor <item>".to_string());
        }
    } else if let Some(r) = reason.as_deref() {
        if !REASONS.contains(&r) {
            return Err(format!(
                "unknown reason '{r}': use one of {}",
                REASONS.join(", ")
            ));
        }
    }
    Ok(Some(Intent {
        action,
        kind,
        item,
        reason,
        reason_text,
        successor,
        repo,
        gh_args,
    }))
}

fn item_number(raw: &str) -> Result<String, String> {
    let tail = raw.trim_end_matches('/').rsplit('/').next().unwrap_or(raw);
    let digits = tail.trim_start_matches('#');
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits.to_string())
    } else {
        Err(format!("cannot read an item number from '{raw}'"))
    }
}

impl Intent {
    /// The gh arguments with the close comment carrying the reason.
    /// Reopen goes through unchanged.
    pub fn gh_args_with_reason(&self) -> Vec<String> {
        let Some(reason) = self.reason.as_deref().filter(|_| self.action == "close") else {
            return self.gh_args.clone();
        };
        let mut body = format!("Closed as {reason}.");
        if let Some(text) = self.reason_text.as_deref().filter(|t| !t.is_empty()) {
            body.push_str(&format!(" {text}"));
        }
        if let Some(s) = self.successor.as_deref().filter(|s| !s.is_empty()) {
            body.push_str(&format!("\nSuccessor: {s}"));
        }
        let mut out = self.gh_args.clone();
        out.push("--comment".to_string());
        out.push(body);
        out
    }

    pub fn record(&self, binding: &Binding, at_unix: u64) -> serde_json::Value {
        serde_json::json!({
            "schema": RECORD_SCHEMA,
            "action": self.action,
            "item": {
                "kind": self.kind,
                "number": self.item.parse::<u64>().ok(),
                "repo": self.repo,
            },
            "reason": self.reason,
            "reason_text": self.reason_text,
            "successor": self.successor,
            "at_unix": at_unix,
            "binding": {
                "run_id": null,
                "worker": binding.worker,
                "lane": binding.lane,
                "task_id": binding.task_id,
                "session_id": binding.session_id,
                "user": binding.user,
            },
        })
    }
}

/// Who is acting, as far as the environment says. Every field is
/// optional; `run_id` stays null until the fleet-init stamp (#9178)
/// lands and is then filled by that stamp rather than here.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Binding {
    pub worker: Option<String>,
    pub lane: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub user: Option<String>,
}

impl Binding {
    /// Build a binding with a session id resolved for this operation. The
    /// caller may provide a live worktree-marker result; `None` keeps the
    /// environment-only behavior for callers without that resolution step.
    pub fn from_resolved_session_id(session_id: Option<String>) -> Binding {
        Self::from_session_id(session_id, true)
    }

    /// Construct a binding after session resolution failed. In that case the
    /// unresolved environment values must not be used as a fallback.
    pub fn from_failed_session_resolution() -> Binding {
        Self::from_session_id(None, false)
    }

    fn from_session_id(session_id: Option<String>, allow_env_fallback: bool) -> Binding {
        let get = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        Binding {
            worker: get("FLEET_WORKER"),
            lane: get("FLEET_LANE"),
            task_id: get("FLEET_TASK_ID"),
            session_id: session_binding(
                session_id,
                allow_env_fallback,
                get("FLEET_SESSION_ID"),
                get("OPENCODE_SESSION_ID"),
            ),
            user: get("USER"),
        }
    }
}

fn preferred_session_id(fleet: Option<String>, opencode: Option<String>) -> Option<String> {
    fleet.or(opencode)
}

fn session_binding(
    resolved: Option<String>,
    allow_env_fallback: bool,
    fleet: Option<String>,
    opencode: Option<String>,
) -> Option<String> {
    resolved.or_else(|| {
        allow_env_fallback
            .then(|| preferred_session_id(fleet, opencode))
            .flatten()
    })
}

pub fn log_path() -> PathBuf {
    match std::env::var("FLEET_CLOSURE_LOG")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(p) => PathBuf::from(p),
        None => Config::state_dir().join(LOG_FILE),
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn append(path: &std::path::Path, record: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{record}")
}

/// Records matching `item` (all records when None), oldest first,
/// one JSON object per line.
pub fn query(log: &str, item: Option<&str>) -> Vec<serde_json::Value> {
    log.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|r| match item {
            None => true,
            Some(n) => {
                r["item"]["number"]
                    .as_u64()
                    .map(|x| x.to_string())
                    .as_deref()
                    == Some(n)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // oracle: the policy text from issue #9221's acceptance, verbatim.
    #[test]
    fn close_without_reason_is_refused_with_policy_text() {
        assert_eq!(
            parse(&s(&["issue", "close", "9221"])),
            Err(POLICY_ERROR.to_string())
        );
        assert_eq!(
            parse(&s(&["pr", "close", "12", "--repo", "a/b"])),
            Err(POLICY_ERROR.to_string())
        );
    }

    // oracle: the four taxonomy values from #9221 each close and keep their value.
    #[test]
    fn each_taxonomy_value_is_accepted() {
        for r in ["implemented", "cancelled", "started-incorrectly"] {
            let i = parse(&s(&["issue", "close", "1", "--reason", r]))
                .unwrap()
                .unwrap();
            assert_eq!(i.reason.as_deref(), Some(r));
        }
        let i = parse(&s(&[
            "issue",
            "close",
            "1",
            "--reason",
            "superseded",
            "--successor",
            "#2",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(i.successor.as_deref(), Some("#2"));
        assert_eq!(i.item, "1");
    }

    #[test]
    fn unknown_reason_is_refused() {
        assert!(parse(&s(&["issue", "close", "1", "--reason", "meh"])).is_err());
    }

    // oracle: #9221 — superseded requires a successor reference.
    #[test]
    fn superseded_without_successor_is_refused() {
        let e = parse(&s(&["pr", "close", "1", "--reason", "superseded"])).unwrap_err();
        assert!(e.contains("--successor"), "{e}");
    }

    #[test]
    fn reopen_needs_no_reason_and_passes_through() {
        let i = parse(&s(&["issue", "reopen", "9221"])).unwrap().unwrap();
        assert_eq!(i.action, "reopen");
        assert_eq!(i.gh_args_with_reason(), s(&["issue", "reopen", "9221"]));
    }

    #[test]
    fn other_gh_calls_are_not_intercepted() {
        assert_eq!(parse(&s(&["issue", "view", "1"])), Ok(None));
        assert_eq!(parse(&s(&["api", "rate_limit"])), Ok(None));
    }

    #[test]
    fn reason_becomes_the_close_comment() {
        let i = parse(&s(&[
            "issue",
            "close",
            "7",
            "--repo",
            "a/b",
            "--reason",
            "superseded",
            "--successor",
            "9",
            "--reason-text",
            "folded in",
        ]))
        .unwrap()
        .unwrap();
        let args = i.gh_args_with_reason();
        assert_eq!(&args[..4], &s(&["issue", "close", "7", "--repo"])[..]);
        assert_eq!(args[4], "a/b");
        assert_eq!(args[5], "--comment");
        assert_eq!(args[6], "Closed as superseded. folded in\nSuccessor: 9");
        assert!(!args.iter().any(|a| a == "--reason" || a == "--successor"));
    }

    // oracle: the record shape the closure log promises.
    #[test]
    fn record_carries_item_reason_time_and_binding() {
        let i = parse(&s(&["issue", "close", "9221", "--reason", "implemented"]))
            .unwrap()
            .unwrap();
        let b = Binding {
            worker: Some("repo-worker-code".into()),
            lane: Some("code-f".into()),
            task_id: Some("20372".into()),
            session_id: None,
            user: None,
        };
        let r = i.record(&b, 1700000000);
        assert_eq!(r["schema"], RECORD_SCHEMA);
        assert_eq!(r["action"], "close");
        assert_eq!(r["item"]["number"], 9221);
        assert_eq!(r["item"]["kind"], "issue");
        assert_eq!(r["reason"], "implemented");
        assert_eq!(r["at_unix"], 1700000000u64);
        assert_eq!(r["binding"]["task_id"], "20372");
        assert!(r["binding"]["run_id"].is_null());
    }

    #[test]
    fn resolved_fleet_session_id_precedes_opencode_for_provenance() {
        assert_eq!(
            preferred_session_id(Some("ses_explicit".into()), Some("ses_open".into())),
            Some("ses_explicit".into())
        );
        assert_eq!(
            preferred_session_id(None, Some("ses_open".into())),
            Some("ses_open".into())
        );
        let binding = Binding::from_resolved_session_id(Some("ses_marker".into()));
        assert_eq!(binding.session_id.as_deref(), Some("ses_marker"));
    }

    #[test]
    fn failed_session_resolution_never_falls_back_to_unvalidated_environment() {
        assert_eq!(
            session_binding(
                None,
                false,
                Some("not-a-session-id".into()),
                Some("ses_open".into())
            ),
            None,
            "resolution failure must not stamp either raw environment value"
        );
        assert_eq!(
            session_binding(
                None,
                true,
                Some("ses_fleet".into()),
                Some("ses_open".into())
            ),
            Some("ses_fleet".into()),
            "ordinary environment-only callers retain their documented fallback"
        );
    }

    #[test]
    fn query_filters_by_item_number() {
        let log = [
            r#"{"item":{"number":9221},"action":"close"}"#,
            r#"{"item":{"number":9222},"action":"close"}"#,
            "not json",
            r#"{"item":{"number":9221},"action":"reopen"}"#,
        ]
        .join("\n");
        let hits = query(&log, Some("9221"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1]["action"], "reopen");
        assert_eq!(query(&log, None).len(), 3);
    }

    #[test]
    fn item_accepts_url_and_hash() {
        assert_eq!(
            item_number("https://github.com/developing-today/code/issues/9221").unwrap(),
            "9221"
        );
        assert_eq!(item_number("#42").unwrap(), "42");
        assert!(item_number("nope").is_err());
    }
}
