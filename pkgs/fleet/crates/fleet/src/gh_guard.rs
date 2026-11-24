//! Bound-task issue close guard for `fleet gh` (#9177).
//!
//! A close (or `status:done` label write) to an issue owned by a
//! claimed/parked queue task requires that task's `fleet-closure/v1`
//! completion record. The record is explicitly presented with
//! `--completion-record <path>`; reads of the queue broker fail open
//! under the standing staleness rule, with the periodic queue sweep
//! providing the detective net.

use serde_json::Value;

const PRESENT_FLAG: &str = "--completion-record";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub issue: String,
    pub repo: Option<String>,
    pub completion_record: Option<String>,
    pub gh_args: Vec<String>,
}

/// Recognize issue close and status:done label writes; all other gh
/// commands pass through untouched. The wrapper-only completion-record
/// flag is removed from the args sent to gh.
pub fn parse(args: &[String]) -> Result<Option<Intent>, String> {
    if args.len() < 3 || args[0] != "issue" {
        return Ok(None);
    }
    let close = args[1] == "close";
    let edit = args[1] == "edit";
    if !close && !edit {
        return Ok(None);
    }
    let mut item = None;
    let mut repo = None;
    let mut record = None;
    let mut guarded_label = false;
    let mut out = vec![args[0].clone(), args[1].clone()];
    let mut i = 2;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == PRESENT_FLAG || arg.starts_with("--completion-record=") {
            let value = if let Some((_, v)) = arg.split_once('=') {
                v.to_string()
            } else {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{PRESENT_FLAG} needs a file path"))?
            };
            if value.trim().is_empty() {
                return Err(format!("{PRESENT_FLAG} needs a non-empty file path"));
            }
            record = Some(value);
            i += 1;
            continue;
        }
        if !arg.starts_with('-') && item.is_none() {
            item = Some(arg.clone());
            out.push(arg.to_string());
            i += 1;
            continue;
        }
        let (name, inline) = arg
            .split_once('=')
            .map_or((arg, None), |(k, v)| (k, Some(v)));
        if matches!(name, "--repo" | "-R") {
            let value = match inline {
                Some(v) => v.to_string(),
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| format!("{name} needs a value"))?
                }
            };
            repo = Some(value.clone());
            out.push(name.to_string());
            out.push(value);
            i += 1;
            continue;
        }
        if edit && matches!(name, "--add-label" | "--label") {
            let value = match inline {
                Some(v) => v.to_string(),
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| format!("{name} needs a value"))?
                }
            };
            guarded_label |= value.split(',').any(|label| label.trim() == "status:done");
            out.push(name.to_string());
            out.push(value);
            i += 1;
            continue;
        }
        // Fleet-owned closure flags and common gh flags take a value;
        // don't mistake a value placed before the issue positional for
        // the issue itself. This also preserves the exact gh argv.
        if matches!(
            name,
            "--reason"
                | "-r"
                | "--reason-text"
                | "--successor"
                | "--milestone"
                | "--assignee"
                | "--project"
                | "--title"
                | "--body"
                | "--comment"
                | "-c"
        ) {
            out.push(name.to_string());
            if let Some(value) = inline {
                out.push(value.to_string());
                i += 1;
            } else {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))?;
                out.push(value);
                i += 1;
            }
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    if !close && !guarded_label {
        return Ok(None);
    }
    let issue = item.ok_or_else(|| "issue close/edit needs an issue number or URL".to_string())?;
    let issue = item_number(&issue)?;
    Ok(Some(Intent {
        issue,
        repo,
        completion_record: record,
        gh_args: out,
    }))
}

fn item_number(raw: &str) -> Result<String, String> {
    let tail = raw.trim_end_matches('/').rsplit('/').next().unwrap_or(raw);
    let digits = tail.trim_start_matches('#');
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits.to_string())
    } else {
        Err(format!("cannot read an issue number from '{raw}'"))
    }
}

fn repo_identity(intent: &Intent) -> Option<String> {
    intent
        .repo
        .clone()
        .or_else(|| {
            std::env::var("GH_REPO")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .or_else(|| {
            let output = std::process::Command::new("git")
                .args(["remote", "get-url", "origin"])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            github_repo_from_remote(std::str::from_utf8(&output.stdout).ok()?.trim())
        })
}

pub(crate) fn github_repo_from_remote(remote: &str) -> Option<String> {
    let path = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let repo = path.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = repo.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

fn task_issue(row: &Value) -> Option<i64> {
    let payload = row.get("payload")?;
    payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(Value::as_i64)
        .or_else(|| {
            let repo = payload.get("repo")?.as_str()?;
            let key = row.get("dedupe_key")?.as_str()?;
            let marker = format!("{repo}:issue:");
            key.split_once(&marker)?
                .1
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
}

fn row_repo(row: &Value) -> Option<&str> {
    row.get("payload")?.get("repo")?.as_str()
}

/// Return the first live binding that owns this repo/issue. A row is
/// bound only when it is claimed or parked AND carries a live session
/// binding (claim_session_id, #9046) — a claimed row with no session
/// stamp is machine-held, not the bound in-progress task this guard
/// exists to protect.
pub fn bound_task(rows: &[Value], repo: &str, issue: &str) -> Option<String> {
    let number = issue.parse::<i64>().ok()?;
    rows.iter()
        .find(|row| {
            matches!(
                row.get("status").and_then(Value::as_str),
                Some("claimed" | "parked")
            ) && !row
                .get("claim_session_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
                && row_repo(row) == Some(repo)
                && task_issue(row) == Some(number)
        })
        .and_then(|row| row.get("id").and_then(Value::as_i64))
        .map(|id| id.to_string())
}

/// A presented completion record must be a close record for this exact
/// issue and bound task. Malformed and unrelated records never bypass.
pub fn record_matches(text: &str, task_id: &str, issue: &str) -> bool {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| {
            record.get("schema").and_then(Value::as_str) == Some("fleet-closure/v1")
                && record.get("action").and_then(Value::as_str) == Some("close")
                && record
                    .get("item")
                    .and_then(|v| v.get("kind"))
                    .and_then(Value::as_str)
                    .map_or(true, |k| k == "issue")
                && record
                    .get("item")
                    .and_then(|v| v.get("number"))
                    .and_then(Value::as_u64)
                    .map(|n| n.to_string())
                    .as_deref()
                    == Some(issue)
                && record
                    .get("binding")
                    .and_then(|v| v.get("task_id"))
                    .and_then(Value::as_str)
                    == Some(task_id)
        })
}

/// Decide the user-visible guard outcome after queue matching. Kept
/// separate from broker I/O so the refusal/bypass oracle is unit-testable.
pub fn enforce(
    task_id: Option<&str>,
    record_text: Option<&str>,
    issue: &str,
    repo: &str,
) -> Result<(), String> {
    let Some(task_id) = task_id else {
        return Ok(());
    };
    if record_text.is_some_and(|text| record_matches(text, task_id, issue)) {
        return Ok(());
    }
    if record_text.is_some() {
        return Err(format!(
            "presented completion record does not match bound task {task_id} and issue {repo}#{issue}"
        ));
    }
    Err(format!(
        "refusing to close {repo}#{issue}: bound queue task {task_id} is still active; present its completion record with {PRESENT_FLAG} <path> (or reopen the issue after resolving the task)"
    ))
}

/// Enforce the guard. Queue broker errors are fail-open but explicit on
/// stderr; they do not turn a permitted GitHub write into a failure.
pub fn check(intent: &Intent) -> Result<(), String> {
    let Some(repo) = repo_identity(intent) else {
        eprintln!(
            "fleet gh: close guard skipped: repository is not explicit (--repo or GH_REPO); fail-open"
        );
        return Ok(());
    };
    #[cfg(feature = "queue")]
    let rows = match fleet_queue::client::guard_rows(500) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("fleet gh: close guard could not query queue broker ({error}); fail-open");
            return Ok(());
        }
    };
    #[cfg(not(feature = "queue"))]
    let rows: Vec<Value> = {
        eprintln!(
            "fleet gh: close guard unavailable in this build (queue client not compiled); fail-open"
        );
        Vec::new()
    };
    let task_id = bound_task(&rows, &repo, &intent.issue);
    let record_text = intent
        .completion_record
        .as_deref()
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read completion record '{path}': {e}"))
        })
        .transpose()?;
    enforce(
        task_id.as_deref(),
        record_text.as_deref(),
        &intent.issue,
        &repo,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // oracle: fleet-queue /list rows use status, payload.repo, issue.number and id.
    #[test]
    fn finds_only_matching_claimed_or_parked_tasks() {
        let rows = vec![
            serde_json::json!({"id":1,"status":"claimed","claim_session_id":"ses_1","payload":{"repo":"a/b","issue":{"number":7}}}),
            serde_json::json!({"id":2,"status":"queued","payload":{"repo":"a/b","issue":{"number":7}}}),
            serde_json::json!({"id":3,"status":"parked","payload":{"repo":"x/y","issue":{"number":7}}}),
        ];
        assert_eq!(bound_task(&rows, "a/b", "7").as_deref(), Some("1"));
        assert_eq!(bound_task(&rows, "a/b", "8"), None);
    }

    // oracle: closure-record bypass is scoped to the fleet-closure/v1 task+issue tuple.
    #[test]
    fn matching_record_bypasses_but_other_records_do_not() {
        let record = r#"{"schema":"fleet-closure/v1","action":"close","item":{"kind":"issue","number":7},"binding":{"task_id":"42"}}"#;
        assert!(record_matches(record, "42", "7"));
        assert!(!record_matches(record, "43", "7"));
        assert!(!record_matches(record, "42", "8"));
        assert!(!record_matches("bad json", "42", "7"));
        let refusal = enforce(Some("42"), None, "7", "a/b").unwrap_err();
        assert!(refusal.contains("bound queue task 42"), "{refusal}");
        assert!(enforce(Some("42"), Some(record), "7", "a/b").is_ok());
        assert!(enforce(None, None, "7", "a/b").is_ok());
    }

    // oracle: #9177 requires closes and status:done label writes to share the guard.
    #[test]
    fn parses_close_and_done_label_and_strips_presented_record_flag() {
        let close = parse(&s(&[
            "issue",
            "close",
            "7",
            "--repo",
            "a/b",
            "--completion-record",
            "/tmp/r",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(close.issue, "7");
        assert_eq!(close.completion_record.as_deref(), Some("/tmp/r"));
        assert!(
            !close
                .gh_args
                .iter()
                .any(|a| a == PRESENT_FLAG || a == "/tmp/r")
        );
        assert!(
            parse(&s(&["issue", "edit", "7", "--add-label", "status:done"]))
                .unwrap()
                .is_some()
        );
        assert!(
            parse(&s(&["issue", "edit", "7", "--add-label", "bug"]))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            github_repo_from_remote("git@github.com:developing-today/code.git").as_deref(),
            Some("developing-today/code")
        );
    }
}
