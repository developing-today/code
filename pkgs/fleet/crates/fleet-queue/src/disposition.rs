//! Verified disposition record (#10611).
//!
//! The broker-side completion gate used to validate only the *shape*
//! of a completion result: a fix-class row with any non-empty
//! evidence string, or a triage exit `already_solved` with any
//! evidence, could be completed `done` while the issue was open and
//! a linked draft PR still carried unfinished work (queue task 15345
//! on code#9155 with draft code#9245 open, completed 2026-11-16).
//!
//! A disposition is the *readback*: what live GitHub said at
//! `checked_at`. The pure gate below decides from that record alone
//! — no network I/O — and `repo-queue verify-disposition` produces
//! one through the fleet's `gh` rotation so the gate stays offline.
//!
//! ## Linked-draft discovery
//!
//! The readback unions three signals for "this issue's PRs":
//!
//! 1. queue provenance — the PR numbers the completion result
//!    carries (`pr`, `pr_url`, `merged_pr`), passed as explicit
//!    linked numbers;
//! 2. an explicit `Refs #n` / `Related program: #n` marker in a PR
//!    body;
//! 3. the `--pr` arguments the operator passes.
//!
//! A bare prose `#n` is *not* a link. code#9245 is linked to #9155
//! only by the prose marker `Related program: #9155`, so a discovery
//! rule that ignored prose markers would never see the draft that
//! keeps #9155 unfinished — the exact miss this module exists to
//! prevent.

use serde_json::{Value, json};

/// Default staleness bound for a disposition's `checked_at`: a
/// readback older than this is not evidence of *current* state.
pub const DEFAULT_STALENESS_SECONDS: i64 = 30 * 60;

/// A readback timestamp further in the future than this is treated
/// as unreadable (clock skew), never as fresh.
pub const FUTURE_SKEW_SECONDS: i64 = 5 * 60;

pub const VERDICTS: [&str; 5] = [
    "closed_verified",
    "merged_verified",
    "open_draft_with_followup",
    "open_draft_superseded",
    "invalid_no_work",
];

/// One pull in the readback.
#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    pub number: i64,
    pub state: String,
    pub is_draft: bool,
    pub merged_at: Option<String>,
}

impl Pull {
    /// A pull that is still carrying work: not merged, not closed.
    /// `merged_at` wins over a stale `state` string.
    fn unfinished(&self) -> bool {
        self.merged_at.is_none() && self.state != "closed"
    }
}

/// A parsed, well-formed disposition record.
#[derive(Debug, Clone, PartialEq)]
pub struct Disposition {
    pub checked_at: String,
    pub checked_at_epoch: i64,
    pub source: String,
    pub issue_number: i64,
    pub issue_state: String,
    pub issue_state_reason: Option<String>,
    pub issue_closed_at: Option<String>,
    pub pulls: Vec<Pull>,
    pub verdict: String,
    pub follow_up: Option<Value>,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}

fn non_empty_str(v: &Value, key: &str) -> Option<String> {
    s(v, key)
}

/// Parse and validate a disposition object. Every rejection names
/// the field that is missing, malformed or unreadable.
pub fn parse_record(v: &Value) -> Result<Disposition, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "disposition is not an object".to_string())?;

    let checked_at = non_empty_str(v, "checked_at")
        .ok_or_else(|| "disposition.checked_at missing".to_string())?;
    let checked_at_epoch = parse_rfc3339_epoch(&checked_at)
        .ok_or_else(|| format!("disposition.checked_at {checked_at:?} is not RFC 3339"))?;
    let source =
        non_empty_str(v, "source").ok_or_else(|| "disposition.source missing".to_string())?;

    let issue = obj
        .get("issue")
        .ok_or_else(|| "disposition.issue missing".to_string())?;
    let issue_number = issue
        .get("number")
        .and_then(|n| n.as_i64())
        .ok_or_else(|| "disposition.issue.number missing".to_string())?;
    let issue_state = non_empty_str(issue, "state")
        .ok_or_else(|| "disposition.issue.state missing".to_string())?;
    if issue_state != "open" && issue_state != "closed" {
        return Err(format!(
            "disposition.issue.state {issue_state:?} is not open|closed"
        ));
    }
    let issue_state_reason = non_empty_str(issue, "state_reason");
    let issue_closed_at = non_empty_str(issue, "closed_at");

    let mut pulls = Vec::new();
    match obj.get("pulls") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items {
                let number = item
                    .get("number")
                    .and_then(|n| n.as_i64())
                    .ok_or_else(|| "disposition.pulls[].number missing".to_string())?;
                let state = non_empty_str(item, "state")
                    .ok_or_else(|| format!("disposition.pulls[{number}].state missing"))?;
                if !matches!(state.as_str(), "open" | "closed" | "merged") {
                    return Err(format!(
                        "disposition.pulls[{number}].state {state:?} is not open|closed|merged"
                    ));
                }
                let is_draft = item
                    .get("is_draft")
                    .and_then(|d| d.as_bool())
                    .unwrap_or(false);
                let merged_at = non_empty_str(item, "merged_at");
                if let Some(m) = &merged_at {
                    if parse_rfc3339_epoch(m).is_none() {
                        return Err(format!(
                            "disposition.pulls[{number}].merged_at {m:?} is not RFC 3339"
                        ));
                    }
                }
                pulls.push(Pull {
                    number,
                    state,
                    is_draft,
                    merged_at,
                });
            }
        }
        Some(_) => return Err("disposition.pulls is not an array".to_string()),
    }

    let verdict =
        non_empty_str(v, "verdict").ok_or_else(|| "disposition.verdict missing".to_string())?;
    if !VERDICTS.contains(&verdict.as_str()) {
        return Err(format!(
            "disposition.verdict {verdict:?} is not one of {}",
            VERDICTS.join("|")
        ));
    }

    let follow_up = match obj.get("follow_up") {
        None | Some(Value::Null) => None,
        Some(f) => {
            let has_target = f.get("task_id").and_then(|t| t.as_i64()).is_some()
                || non_empty_str(f, "issue_url").is_some();
            let reason = non_empty_str(f, "reason");
            if !has_target {
                return Err("disposition.follow_up needs task_id or issue_url".to_string());
            }
            if reason.is_none() {
                return Err("disposition.follow_up.reason missing".to_string());
            }
            Some(f.clone())
        }
    };

    Ok(Disposition {
        checked_at,
        checked_at_epoch,
        source,
        issue_number,
        issue_state,
        issue_state_reason,
        issue_closed_at,
        pulls,
        verdict,
        follow_up,
    })
}

/// Decide whether `record` supports a terminal completion.
///
/// Rules (the declared oracle, exercised by the replay tests):
///
/// - the record must be fresh: `0 <= now - checked_at <= staleness`;
/// - it must describe the issue the packet names;
/// - `closed_verified` needs a CLOSED issue — a close timestamp while
///   the issue is open is a superseded close, not closure evidence
///   (the #9155 01:45 close / 02:02 reopen shape);
/// - `merged_verified` needs a merged pull;
/// - the `open_draft_*` verdicts need a recorded follow-up;
/// - every PR the completion result links must appear in the
///   readback — an uncovered linked PR is an unchecked PR;
/// - while the issue is OPEN, unfinished linked pulls are only
///   allowed under `open_draft_with_followup` / `open_draft_superseded`
///   with that follow-up on the record.
pub fn check(
    record: &Disposition,
    expected_issue: Option<i64>,
    linked_prs: &[i64],
    now_epoch: i64,
    staleness_seconds: i64,
) -> Result<(), String> {
    if record.checked_at_epoch > now_epoch + FUTURE_SKEW_SECONDS {
        return Err(format!(
            "disposition.checked_at {:?} is in the future; unreadable \
             readback",
            record.checked_at
        ));
    }
    let age = now_epoch - record.checked_at_epoch;
    if age > staleness_seconds {
        return Err(format!(
            "disposition.checked_at {:?} is {}s old (bound {}s); re-run \
             `repo-queue verify-disposition`",
            record.checked_at, age, staleness_seconds
        ));
    }
    if let Some(expected) = expected_issue {
        if record.issue_number != expected {
            return Err(format!(
                "disposition describes issue #{} but the task is for #{expected}",
                record.issue_number
            ));
        }
    }

    match record.verdict.as_str() {
        "closed_verified" => {
            if record.issue_state != "closed" {
                return Err(format!(
                    "verdict closed_verified but issue #{} is {}; a close \
                     older than the latest reopen is not closure evidence",
                    record.issue_number, record.issue_state
                ));
            }
        }
        "merged_verified" => {
            if !record.pulls.iter().any(|p| p.merged_at.is_some()) {
                return Err(format!(
                    "verdict merged_verified but no pull in the readback is merged \
                     (issue #{})",
                    record.issue_number
                ));
            }
        }
        "open_draft_with_followup" | "open_draft_superseded" => {
            if record.follow_up.is_none() {
                return Err(format!(
                    "verdict {} needs a recorded follow_up (task_id or \
                     issue_url plus reason)",
                    record.verdict
                ));
            }
        }
        _ => {}
    }

    for number in linked_prs {
        if !record.pulls.iter().any(|p| p.number == *number) {
            return Err(format!(
                "linked PR #{number} is not in the readback; the disposition \
                 must cover every PR the completion claims (issue #{})",
                record.issue_number
            ));
        }
    }

    if record.issue_state == "open" {
        let unfinished: Vec<i64> = record
            .pulls
            .iter()
            .filter(|p| p.unfinished())
            .map(|p| p.number)
            .collect();
        if !unfinished.is_empty()
            && !matches!(
                record.verdict.as_str(),
                "open_draft_with_followup" | "open_draft_superseded"
            )
        {
            return Err(format!(
                "issue #{} is OPEN with open/unmerged linked PR {}; a terminal \
                 exit needs verdict open_draft_with_followup or \
                 open_draft_superseded plus a recorded follow_up",
                record.issue_number,
                unfinished
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}

/// The disposition carried by a completion result, parsed and
/// freshness-checked. Rejections are the gate's wording.
pub fn require_verified(
    result: &Value,
    expected_issue: Option<i64>,
    now_epoch: i64,
) -> Result<Disposition, String> {
    let raw = result.get("disposition").ok_or_else(|| {
        "complete_task: this completion requires a verified disposition \
             (result.disposition: checked_at, source, issue, pulls, verdict); \
             run `repo-queue verify-disposition <issue> --json`, add \
             follow_up when work remains, and complete again — the task \
             remains claimed"
            .to_string()
    })?;
    let record = parse_record(raw).map_err(|e| {
        format!(
            "complete_task: result.disposition is not well-formed ({e}); the \
             task remains claimed"
        )
    })?;
    let linked = linked_prs(result);
    check(
        &record,
        expected_issue,
        &linked,
        now_epoch,
        DEFAULT_STALENESS_SECONDS,
    )
    .map_err(|e| format!("complete_task: {e}; the task remains claimed"))?;
    Ok(record)
}

/// PR numbers a completion result claims as its artifact: the
/// structured fields the fleet's completion notes use.
pub fn linked_prs(result: &Value) -> Vec<i64> {
    let mut out: Vec<i64> = Vec::new();
    let mut push = |n: i64| {
        if !out.contains(&n) {
            out.push(n);
        }
    };
    for key in [
        "pr",
        "pr_url",
        "merged_pr",
        "pull_request",
        "evidence",
        "commit",
        "commit_sha",
    ] {
        match result.get(key) {
            Some(v) => collect_pr_numbers(v, &mut push),
            None => {}
        }
    }
    // A bare `pr: 9245` counts too.
    if let Some(n) = result.get("pr").and_then(|p| p.as_i64()) {
        push(n);
    }
    out
}

fn collect_pr_numbers(v: &Value, push: &mut impl FnMut(i64)) {
    match v {
        Value::String(s) => {
            if let Some(n) = pr_number_in(s) {
                push(n);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_pr_numbers(item, push);
            }
        }
        Value::Object(map) => {
            if let Some(n) = map.get("number").and_then(|n| n.as_i64()) {
                // `{"number": 9245}` only counts next to a PR-ish
                // shape; a bare object number is the fleet's pr field.
                push(n);
            }
            for (k, val) in map {
                if k != "number" {
                    collect_pr_numbers(val, push);
                }
            }
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                push(i);
            }
        }
        _ => {}
    }
}

/// Extract a PR number from a GitHub pull URL or `owner/repo#N`.
pub fn pr_number_in(text: &str) -> Option<i64> {
    let t = text.trim();
    if let Some(idx) = t.find("/pull/") {
        let rest = &t[idx + "/pull/".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().ok();
    }
    // owner/repo#123
    if let Some((_, tail)) = t.rsplit_once('#') {
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.len() == tail.trim().len() {
            return digits.parse().ok();
        }
    }
    None
}

/// Does a PR body carry an explicit link marker for `issue`?
/// `Refs #n` / `Fixes #n` / `Closes #n` / `Related program: #n`
/// count; a bare prose `#n` does not.
pub fn body_links_issue(body: &str, issue: i64) -> bool {
    let needle = format!("#{issue}");
    let lower = body.to_lowercase();
    for marker in ["refs", "fixes", "closes", "related program:", "related:"] {
        let mut start = 0;
        while let Some(idx) = lower[start..].find(marker) {
            let at = start + idx + marker.len();
            let after = lower[at..].trim_start();
            if let Some(rest) = after.strip_prefix(&needle) {
                // `#10611` must not match the marker for `#1061`.
                let next = rest.chars().next();
                if !matches!(next, Some(c) if c.is_ascii_digit()) {
                    return true;
                }
            }
            start = at;
        }
    }
    false
}

/// Assemble a disposition from live GitHub JSON.
///
/// `issue_json` is `gh issue view --json number,state,stateReason,closedAt`;
/// `prs_json` is `gh pr list --state all --json number,state,isDraft,mergedAt,body`.
/// `linked` are the explicit PR numbers (queue provenance or `--pr`);
/// `follow_up` is the operator's recorded follow-up, if any.
pub fn from_gh(
    issue_json: &Value,
    prs_json: &Value,
    linked: &[i64],
    follow_up: Option<Value>,
    checked_at: &str,
) -> Value {
    let issue_number = issue_json
        .get("number")
        .and_then(|n| n.as_i64())
        .unwrap_or(0);
    let state = issue_json
        .get("state")
        .and_then(|x| x.as_str())
        .map(|x| x.to_lowercase())
        .unwrap_or_else(|| "open".to_string());
    let mut pulls = Vec::new();
    if let Some(items) = prs_json.as_array() {
        for item in items {
            let number = match item.get("number").and_then(|n| n.as_i64()) {
                Some(n) => n,
                None => continue,
            };
            let body = item.get("body").and_then(|b| b.as_str()).unwrap_or("");
            let keep = linked.contains(&number) || body_links_issue(body, issue_number);
            if !keep {
                continue;
            }
            pulls.push(json!({
                "number": number,
                "state": item.get("state").and_then(|x| x.as_str()).unwrap_or("OPEN").to_lowercase(),
                "is_draft": item.get("isDraft").and_then(|d| d.as_bool()).unwrap_or(false),
                "merged_at": item.get("mergedAt").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    let merged = pulls
        .iter()
        .any(|p| p.get("merged_at").map(|m| !m.is_null()).unwrap_or(false));
    let unfinished = pulls.iter().any(|p| {
        let merged_at = p.get("merged_at").cloned().unwrap_or(Value::Null);
        let state = p.get("state").and_then(|x| x.as_str()).unwrap_or("");
        merged_at.is_null() && state != "closed"
    });
    let verdict = if merged {
        "merged_verified"
    } else if state == "closed" && !unfinished {
        "closed_verified"
    } else if unfinished {
        "open_draft_with_followup"
    } else {
        "invalid_no_work"
    };
    json!({
        "checked_at": checked_at,
        "source": "fleet gh readback",
        "issue": {
            "number": issue_number,
            "state": state,
            "state_reason": issue_json.get("stateReason").cloned().unwrap_or(Value::Null),
            "closed_at": issue_json.get("closedAt").cloned().unwrap_or(Value::Null),
        },
        "pulls": pulls,
        "verdict": verdict,
        "follow_up": follow_up.unwrap_or(Value::Null),
    })
}

// ---------------------------------------------------------------------------
// RFC 3339 -> epoch seconds (compact; the gate needs no formatting)
// ---------------------------------------------------------------------------

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` to epoch seconds.
/// The day must exist in its month (no silent normalization).
pub fn parse_rfc3339_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s
        .split_once('T')
        .or_else(|| s.split_once('t'))
        .or_else(|| s.split_once(' '))?;
    let mut dp = date.split('-');
    fn digits(s: &str, n: usize) -> Option<i64> {
        if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    }
    let year: i64 = digits(dp.next()?, 4)?;
    let month: u32 = digits(dp.next()?, 2)?.try_into().ok()?;
    let day: u32 = digits(dp.next()?, 2)?.try_into().ok()?;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if dp.next().is_some() || day < 1 || day > days_in_month {
        return None;
    }
    let (time, offset_s) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0i64)
    } else if let Some(idx) = rest.rfind(['+', '-']) {
        if idx == 0 {
            return None;
        }
        let (t, off) = rest.split_at(idx);
        let sign: i64 = if rest.as_bytes()[idx] == b'+' { 1 } else { -1 };
        let mut op = off[1..].split(':');
        let oh: i64 = digits(op.next()?, 2)?;
        let om: i64 = digits(op.next()?, 2)?;
        if op.next().is_some() || oh > 23 || om > 59 {
            return None;
        }
        (t, sign * (oh * 3600 + om * 60))
    } else {
        (rest, 0)
    };
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, b),
        None => (time, ""),
    };
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut tp = hms.split(':');
    let hh: i64 = digits(tp.next()?, 2)?;
    let mm: i64 = digits(tp.next()?, 2)?;
    let ss: i64 = digits(tp.next()?, 2)?;
    if tp.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hh * 3600 + mm * 60 + ss - offset_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_issue_disposition(checked_at: &str) -> Value {
        json!({
            "checked_at": checked_at,
            "source": "fleet gh readback",
            "issue": {"number": 10611, "state": "closed", "closed_at": checked_at},
            "pulls": [{"number": 9245, "state": "closed", "is_draft": false,
                       "merged_at": "2026-11-16T05:00:00Z"}],
            "verdict": "merged_verified",
        })
    }

    const NOW: i64 = 1_794_807_540;

    #[test]
    fn parse_and_check_clean_records() {
        let rec = parse_record(&json!({
            "checked_at": "2026-11-16T05:39:00Z",
            "source": "fleet gh readback",
            "issue": {"number": 1, "state": "closed", "closed_at": "2026-11-16T05:00:00Z"},
            "pulls": [{"number": 2, "state": "closed", "is_draft": false,
                       "merged_at": "2026-11-16T05:00:00Z"}],
            "verdict": "merged_verified",
        }))
        .expect("well-formed");
        assert_eq!(rec.issue_state, "closed");
        assert_eq!(rec.pulls.len(), 1);
        assert!(
            check(
                &rec,
                Some(1),
                &[2],
                rec.checked_at_epoch + 60,
                DEFAULT_STALENESS_SECONDS
            )
            .is_ok()
        );
    }

    #[test]
    fn missing_or_malformed_disposition_rejects() {
        assert!(parse_record(&json!(null)).is_err());
        assert!(parse_record(&json!({})).is_err());
        // no verdict
        let mut v = closed_issue_disposition("2026-11-16T05:39:00Z");
        v.as_object_mut().unwrap().remove("verdict");
        assert!(parse_record(&v).is_err());
        // unknown verdict
        let mut v = closed_issue_disposition("2026-11-16T05:39:00Z");
        v["verdict"] = json!("looks_fine");
        assert!(parse_record(&v).is_err());
        // bad state
        let mut v = closed_issue_disposition("2026-11-16T05:39:00Z");
        v["issue"]["state"] = json!("OPEN");
        assert!(parse_record(&v).is_err());
        // follow-up without reason
        let mut v = closed_issue_disposition("2026-11-16T05:39:00Z");
        v["verdict"] = json!("open_draft_with_followup");
        v["follow_up"] = json!({"task_id": 7});
        assert!(parse_record(&v).is_err());
    }

    #[test]
    fn stale_and_future_readbacks_reject() {
        let rec = parse_record(&closed_issue_disposition("2026-11-16T05:39:00Z")).unwrap();
        let checked = rec.checked_at_epoch;
        // fresh
        assert!(check(&rec, None, &[], checked + 60, DEFAULT_STALENESS_SECONDS).is_ok());
        // stale by one second past the bound
        assert!(
            check(
                &rec,
                None,
                &[],
                checked + DEFAULT_STALENESS_SECONDS + 1,
                DEFAULT_STALENESS_SECONDS
            )
            .is_err()
        );
        // future beyond skew
        assert!(
            check(
                &rec,
                None,
                &[],
                checked - FUTURE_SKEW_SECONDS - 1,
                DEFAULT_STALENESS_SECONDS
            )
            .is_err()
        );
    }

    #[test]
    fn wrong_issue_rejects() {
        let rec = parse_record(&closed_issue_disposition("2026-11-16T05:39:00Z")).unwrap();
        let err = check(
            &rec,
            Some(9155),
            &[],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .unwrap_err();
        assert!(err.contains("9155"), "{err}");
    }

    #[test]
    fn replay_9155_9245_open_draft_rejected() {
        // The production replay: #9155 reopened, draft #9245 open at
        // 649eba6a / 6bab2429. A close_verified claim is not evidence
        // and an open draft needs a recorded follow-up.
        let mut v = closed_issue_disposition("2026-11-16T05:39:00Z");
        v["issue"]["number"] = json!(9155);
        v["issue"]["state"] = json!("open");
        v["issue"]["closed_at"] = json!("2026-11-13T01:45:20Z");
        v["pulls"] = json!([{"number": 9245, "state": "open", "is_draft": true,
                             "merged_at": null}]);
        // a close timestamp while the issue is open is superseded
        v["verdict"] = json!("closed_verified");
        let rec = parse_record(&v).unwrap();
        let err = check(
            &rec,
            Some(9155),
            &[9245],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .unwrap_err();
        assert!(err.contains("closed_verified"), "{err}");

        // the declared disposition (open draft + follow-up) passes
        v["verdict"] = json!("open_draft_with_followup");
        v["follow_up"] =
            json!({"task_id": 17413, "reason": "#9245 still carries the #9155 design work"});
        let rec = parse_record(&v).unwrap();
        check(
            &rec,
            Some(9155),
            &[9245],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .expect("intentional disposition with a recorded follow-up");

        // same state with no follow-up is a rejection on the open-PR rule
        v["verdict"] = json!("merged_verified");
        v["follow_up"] = Value::Null;
        let rec = parse_record(&v).unwrap();
        assert!(
            check(
                &rec,
                Some(9155),
                &[9245],
                rec.checked_at_epoch,
                DEFAULT_STALENESS_SECONDS
            )
            .is_err()
        );

        // open draft under open_draft_with_followup but follow_up gone
        v["verdict"] = json!("open_draft_with_followup");
        let rec = parse_record(&v).unwrap();
        let err = check(
            &rec,
            Some(9155),
            &[9245],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .unwrap_err();
        assert!(err.contains("follow_up"), "{err}");
    }

    #[test]
    fn uncovered_linked_pr_rejects() {
        let rec = parse_record(&closed_issue_disposition("2026-11-16T05:39:00Z")).unwrap();
        let err = check(
            &rec,
            None,
            &[9245, 9999],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .unwrap_err();
        assert!(err.contains("#9999"), "{err}");
    }

    #[test]
    fn linked_pr_extraction_ignores_prose() {
        assert_eq!(
            linked_prs(&json!({"pr": {"number": 9245, "url": "https://x/pull/9245"}})),
            vec![9245]
        );
        assert_eq!(
            linked_prs(&json!({"pr_url": "https://github.com/o/r/pull/77"})),
            vec![77]
        );
        assert!(linked_prs(&json!({"note": "see #9155 for context"})).is_empty());
    }

    #[test]
    fn body_markers() {
        assert!(body_links_issue("Related program: #9155", 9155));
        assert!(body_links_issue("Refs #9155", 9155));
        assert!(body_links_issue("refs  #9155\n", 9155));
        assert!(!body_links_issue("part of #9155 work", 9155));
        // prefix must not match a longer number
        assert!(!body_links_issue("Refs #91550", 9155));
    }

    #[test]
    fn from_gh_discovers_the_prose_linked_draft() {
        let issue = json!({"number": 9155, "state": "OPEN", "stateReason": null,
                           "closedAt": null});
        let prs = json!([
            {"number": 9245, "state": "OPEN", "isDraft": true, "mergedAt": null,
             "body": "Related program: #9155"},
            {"number": 9100, "state": "OPEN", "isDraft": false, "mergedAt": null,
             "body": "unrelated cleanup"},
        ]);
        let out = from_gh(&issue, &prs, &[], None, "2026-11-16T05:39:00Z");
        assert_eq!(out["pulls"].as_array().unwrap().len(), 1);
        assert_eq!(out["pulls"][0]["number"], 9245);
        assert_eq!(out["verdict"], "open_draft_with_followup");
        // the gate rejects it until a follow-up is recorded
        let rec = parse_record(&out).unwrap();
        assert!(
            check(
                &rec,
                Some(9155),
                &[9245],
                rec.checked_at_epoch,
                DEFAULT_STALENESS_SECONDS
            )
            .is_err()
        );
    }

    #[test]
    fn from_gh_merged_successor_clears_the_draft() {
        let issue = json!({"number": 9155, "state": "CLOSED", "stateReason": "COMPLETED",
                           "closedAt": "2026-11-16T05:00:00Z"});
        let prs = json!([
            {"number": 9245, "state": "OPEN", "isDraft": true, "mergedAt": null,
             "body": "Related program: #9155"},
            {"number": 9300, "state": "MERGED", "isDraft": false,
             "mergedAt": "2026-11-16T05:00:00Z", "body": "Refs #9155"},
        ]);
        let out = from_gh(&issue, &prs, &[], None, "2026-11-16T05:39:00Z");
        assert_eq!(out["verdict"], "merged_verified");
        let rec = parse_record(&out).unwrap();
        check(
            &rec,
            Some(9155),
            &[9245, 9300],
            rec.checked_at_epoch,
            DEFAULT_STALENESS_SECONDS,
        )
        .expect("a merged successor plus the closed issue is a verified disposition");
    }

    #[test]
    fn rfc3339_parse_and_reject_impossible_dates() {
        assert_eq!(parse_rfc3339_epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_epoch("1970-01-01T00:00:00+01:00"),
            Some(-3600)
        );
        assert_eq!(parse_rfc3339_epoch("2026-11-16T05:39:00.123Z"), Some(NOW));
        assert_eq!(parse_rfc3339_epoch("2026-02-31T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_epoch("not a date"), None);
        assert_eq!(parse_rfc3339_epoch("2026-13-01T00:00:00Z"), None);
    }
}
