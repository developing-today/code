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
//!
//! The close comment's free text comes from `--reason-text`. gh's
//! native `--comment`/`-c` is accepted as an alias of `--reason-text`
//! on a close (dezren39/fleet#118) — the composed comment is the
//! wrapper's own, built from the reason, so a caller's comment text
//! folds into it; the two flags combine in argument order when both
//! are given. On a reopen there is no composed comment, so
//! `--comment` forwards to gh unchanged, as real gh accepts it there.
//!
//! The composed comment is also accepted back as input (#16150):
//! when a close carries no `--reason` but its folded comment text is
//! exactly a composed comment (`Closed as <reason>.` plus the
//! optional free text and `Successor:` line this layer generates),
//! the reason, text and successor are recovered from it. Fleet's own
//! generated arguments therefore survive re-entering a fleet — via
//! the gh-fleet shim under a build without the #14650 recursion
//! guard, or any mixed-revision pairing — instead of being refused
//! with the policy error meant for a caller who gave no reason.

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
                "--reason-text" => append_reason_text(&mut reason_text, value),
                "--successor" => successor = Some(value),
                "--repo" | "-R" => {
                    repo = Some(value.clone());
                    gh_args.push(name.to_string());
                    gh_args.push(value);
                }
                // gh's native comment flag (dezren39/fleet#118): on a
                // close it is an alias of --reason-text — the text
                // folds into the composed close comment; on a reopen
                // it forwards to gh, which accepts it there.
                "--comment" | "-c" => {
                    if action == "reopen" {
                        gh_args.push("--comment".to_string());
                        gh_args.push(value);
                    } else {
                        append_reason_text(&mut reason_text, value);
                    }
                }
                _ => return Err(format!("flag '{name}' is not accepted on a closure")),
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
    // #16150: a close that re-enters fleet carrying this layer's own
    // composed comment (see the module header) has no --reason flag —
    // the reason lives in the comment. Recover it before the policy
    // check so the generated form is judged by the same rules as the
    // hand-written one, superseded-needs-successor included.
    if action == "close" && reason.is_none() {
        if let Some((r, text, succ)) = reason_text.as_deref().and_then(decompose_composed_comment) {
            reason = Some(r);
            reason_text = text;
            if successor.is_none() {
                successor = succ;
            }
        }
    }
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

/// Fold one piece of close text into the reason text. `--reason-text`
/// and gh's `--comment` both land here; when both are given (or one is
/// repeated) the pieces join in argument order with a single space.
fn append_reason_text(slot: &mut Option<String>, value: String) {
    match slot.as_mut() {
        Some(existing) if !existing.is_empty() && !value.is_empty() => {
            existing.push(' ');
            existing.push_str(&value);
        }
        _ => {
            if slot.as_deref().is_none_or(str::is_empty) {
                *slot = Some(value);
            }
        }
    }
}

/// Split a composed close comment back into its parts, when `text`
/// is exactly one: `Closed as <reason>.` followed by nothing, by
/// ` <free text>`, by `\nSuccessor: <item>`, or by both in that
/// order — the shapes `Intent::gh_args_with_reason` generates. Any
/// other text returns None, so an ordinary caller comment is never
/// mistaken for a reason. The recovered reason must be a taxonomy
/// value; the caller re-validates the triple as usual.
fn decompose_composed_comment(text: &str) -> Option<(String, Option<String>, Option<String>)> {
    let rest = text.strip_prefix("Closed as ")?;
    let (reason, mut rest) = rest.split_once('.')?;
    if !REASONS.contains(&reason) {
        return None;
    }
    let mut successor = None;
    if let Some((head, tail)) = rest.split_once("\nSuccessor: ") {
        if tail.is_empty() {
            return None;
        }
        successor = Some(tail.to_string());
        rest = head;
    }
    let reason_text = if rest.is_empty() {
        None
    } else if let Some(free) = rest.strip_prefix(' ') {
        if free.is_empty() {
            return None;
        }
        Some(free.to_string())
    } else {
        return None;
    };
    Some((reason.to_string(), reason_text, successor))
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

    // oracle: #16150 — fleet's own generated close arguments (the
    // composed --comment, no --reason flag) parse back to the same
    // intent instead of dying with POLICY_ERROR when they re-enter a
    // fleet through the gh-fleet shim or a mixed-revision pairing.
    #[test]
    fn generated_close_args_round_trip_through_parse() {
        let first = parse(&s(&[
            "pr",
            "close",
            "15772",
            "--repo",
            "developing-today/code",
            "--reason",
            "superseded",
            "--reason-text",
            "probe",
            "--successor",
            "15902",
        ]))
        .unwrap()
        .unwrap();
        let generated = first.gh_args_with_reason();
        let second = parse(&generated).unwrap().unwrap();
        assert_eq!(second.reason.as_deref(), Some("superseded"));
        assert_eq!(second.reason_text.as_deref(), Some("probe"));
        assert_eq!(second.successor.as_deref(), Some("15902"));
        assert_eq!(second.item, "15772");
        // Idempotent: recomposing the recovered intent reproduces the
        // generated arguments exactly — one --comment, no doubling.
        assert_eq!(second.gh_args_with_reason(), generated);
    }

    // oracle: #16150 — the bare and successor-only composed shapes
    // round-trip too, and a superseded shape keeps its successor
    // requirement satisfied from the comment alone.
    #[test]
    fn composed_comment_shapes_are_recovered() {
        let i = parse(&s(&[
            "issue",
            "close",
            "7",
            "--comment",
            "Closed as implemented.",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(i.reason.as_deref(), Some("implemented"));
        assert_eq!(i.reason_text, None);
        let i = parse(&s(&[
            "pr",
            "close",
            "9",
            "--comment",
            "Closed as superseded.\nSuccessor: 10",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(i.reason.as_deref(), Some("superseded"));
        assert_eq!(i.successor.as_deref(), Some("10"));
    }

    // oracle: #16150 — recovery is exact-shape only: an ordinary
    // comment that merely mentions a closing, or names a non-taxonomy
    // reason, is not a reason and still hits the policy error.
    #[test]
    fn ordinary_comments_are_not_recovered_as_reasons() {
        for comment in [
            "Closed as bogus.",
            "closed as implemented.",
            "Closed as implemented",
            "Please close: Closed as implemented. thanks",
        ] {
            assert_eq!(
                parse(&s(&["issue", "close", "7", "--comment", comment])),
                Err(POLICY_ERROR.to_string()),
                "{comment}"
            );
        }
    }

    // oracle: dezren39/fleet#118 — gh's native --comment on a close is
    // the reason text, not a refusal.
    #[test]
    fn comment_flag_is_the_reason_text_on_a_close() {
        let i = parse(&s(&[
            "issue",
            "close",
            "7",
            "--repo",
            "a/b",
            "--reason",
            "implemented",
            "--comment",
            "evidence here",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(i.reason_text.as_deref(), Some("evidence here"));
        let args = i.gh_args_with_reason();
        assert_eq!(
            args.last().map(String::as_str),
            Some("Closed as implemented. evidence here")
        );
        // exactly one --comment reaches gh: the composed one.
        assert_eq!(args.iter().filter(|a| *a == "--comment").count(), 1);
    }

    // oracle: dezren39/fleet#118 — -c and --reason-text combine in
    // argument order when both are given.
    #[test]
    fn short_comment_flag_and_reason_text_combine_in_order() {
        let i = parse(&s(&[
            "pr",
            "close",
            "3",
            "--reason",
            "cancelled",
            "-c",
            "first",
            "--reason-text",
            "second",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(i.reason_text.as_deref(), Some("first second"));
        let args = i.gh_args_with_reason();
        assert_eq!(
            args.last().map(String::as_str),
            Some("Closed as cancelled. first second")
        );
    }

    // oracle: gh accepts --comment on a reopen natively, so the
    // wrapper forwards it instead of folding or refusing it.
    #[test]
    fn comment_forwards_to_gh_on_a_reopen() {
        let i = parse(&s(&["issue", "reopen", "7", "--comment", "back again"]))
            .unwrap()
            .unwrap();
        assert_eq!(i.reason_text, None);
        assert_eq!(
            i.gh_args_with_reason(),
            s(&["issue", "reopen", "7", "--comment", "back again"])
        );
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
