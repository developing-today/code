//! turso_queue::work_identity — open-collision suppression at enqueue
//! (#10627).
//!
//! The 2026-11-16 outcome-taxonomy measurement found 630 issues
//! carrying two or more non-triage tasks, 155 sibling_done
//! completions in a week (the work landed under another task while
//! this one ran) and 86 same-issue pairs with overlapping claim
//! windows. The per-queue `dedupe_key` check did not catch them:
//! generators key the same work differently, and the one-queue
//! model files the same issue into more than one stream.
//!
//! Enqueue now also computes the packet's work identity —
//! (repo, issue, kind family, and for a PR shepherd the PRs it
//! shepherds) — and when a live task already holds that identity
//! in any queue, files no new row: the reply carries the existing
//! task (like a dedupe_key hit) plus a `work_identity_collision`
//! naming its id, state and identity, so a generator can route its
//! follow-up onto it. Failed and cancelled tasks never block a new
//! filing (re-queue after failure is unaffected). A COMPLETED task
//! does block (#13091): the claim-time staleness check (#10626)
//! fails open by standing rule, so during a probe degradation
//! completed work was served again and completed a second time;
//! the broker knows its own verified completions locally and
//! absorbs the re-filing itself, no probe involved.
//!
//! One completed kind is not completed issue work (#16007): a
//! triage task completes by filing its fix follow-on, so when that
//! follow-on later fails or is cancelled (or never landed) and no
//! live or completed follow-on covers the issue, a completed triage
//! row must NOT absorb the reconciliation re-filing in either the
//! dedupe-key check or the completed-identity check below — the
//! fresh triage task is the recovery path, and `/coverage` (live
//! rows only) must be able to see it afterwards.
//!
//! Only kinds that work one issue one way take part: small_fix,
//! triage, and pr_shepherd (per PR set). ci_investigation,
//! second_pass and feature_slice are recurring by design and file
//! as before — a feature_slice parent issue carries many slices in
//! parallel, one dedupe key per slice ordinal (#12233 F2c). A packet
//! may declare `dedupe_exempt: true` (a recurring kind) or
//! `dedupe_override: "<reason>"` (a deliberate parallel, recorded
//! as a `dedupe_override` event on the new task). A quarantined
//! (invalid, blocked) row never absorbs a new filing: the repaired
//! packet files.

use super::*;

/// One packet's work identity (#10627).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkIdentity {
    pub repo: String,
    pub issue: i64,
    pub family: String,
    /// Canonical prompt-store ID; set only for prompt tasks.
    pub prompt_id: Option<String>,
    /// The shepherded PRs, sorted; empty for other families.
    pub prs: Vec<i64>,
}

impl WorkIdentity {
    pub fn key(&self) -> String {
        if let Some(prompt_id) = &self.prompt_id {
            return format!("{}#{}", self.repo, prompt_id);
        }
        let mut key = format!("{}#{}:{}", self.repo, self.issue, self.family);
        if !self.prs.is_empty() {
            let prs: Vec<String> = self.prs.iter().map(i64::to_string).collect();
            key.push_str(&format!(":pr{}", prs.join(",")));
        }
        key
    }
}

/// The kind family a packet kind files under, or None for kinds
/// that recur on one issue by design.
pub fn kind_family(kind: &str) -> Option<&'static str> {
    match kind {
        "small_fix" => Some("fix"),
        "triage" => Some("triage"),
        "pr_shepherd" => Some("pr_shepherd"),
        "prompt" => Some("prompt"),
        _ => None,
    }
}

/// The work identity of a task row or packet (its `payload`, plus
/// the queue for a payload with no repo). None when the packet has
/// no issue, its kind takes no part, or it declares an exemption.
pub fn work_identity(row: &Value) -> Option<WorkIdentity> {
    let payload = task_payload(row);
    if payload.get("dedupe_exempt").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let kind = payload.get("kind").and_then(|v| v.as_str())?;
    let family = kind_family(kind)?;
    let prompt_id = if kind == "prompt" {
        Some(
            payload
                .pointer("/prompt/id")
                .and_then(Value::as_str)?
                .to_string(),
        )
    } else {
        None
    };
    let issue = if kind == "prompt" {
        0
    } else {
        payload
            .get("issue")
            .and_then(|i| i.get("number"))
            .and_then(|n| n.as_i64())
            .filter(|n| *n > 0)?
    };
    let repo = task_repo_identity(row);
    if !repo.contains('/') {
        return None;
    }
    let mut prs: Vec<i64> = Vec::new();
    if family == "pr_shepherd" {
        let mut push = |v: &Value| {
            let n = match v {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => crate::disposition::pr_number_in(s),
                Value::Object(o) => o.get("number").and_then(|n| n.as_i64()).or_else(|| {
                    o.get("url")
                        .and_then(|u| u.as_str())
                        .and_then(crate::disposition::pr_number_in)
                }),
                _ => None,
            };
            if let Some(n) = n.filter(|n| *n > 0) {
                if !prs.contains(&n) {
                    prs.push(n);
                }
            }
        };
        if let Some(pr) = payload.get("pr") {
            push(pr);
        }
        if let Some(list) = payload.get("prs").and_then(|v| v.as_array()) {
            list.iter().for_each(&mut push);
        }
        prs.sort_unstable();
    }
    Some(WorkIdentity {
        repo,
        issue,
        family: family.to_string(),
        prompt_id,
        prs,
    })
}

/// The packet's `dedupe_override` reason, when it names one.
pub fn dedupe_override(packet: &Value) -> Option<String> {
    let payload = packet.get("payload").unwrap_or(packet);
    payload
        .get("dedupe_override")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
}

/// Whether a row in one of `statuses` holds `identity` and may
/// absorb a new filing: same identity, and not a quarantined
/// (invalid) row.
fn holds_identity_in(row: &Value, identity: &WorkIdentity, statuses: &[&str]) -> bool {
    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if !statuses.contains(&status) {
        return false;
    }
    if work_identity(row).as_ref() != Some(identity) {
        return false;
    }
    claim_guard_reason(row).is_none()
}

/// Whether a live row holds `identity` and may absorb a new filing:
/// same identity, and not a quarantined (invalid) row.
pub fn holds_identity(row: &Value, identity: &WorkIdentity) -> bool {
    holds_identity_in(row, identity, &LIVE_STATUSES)
}

/// Whether a completed row holds `identity` (#13091).
fn holds_completed_identity(row: &Value, identity: &WorkIdentity) -> bool {
    holds_identity_in(row, identity, &["done"])
}

/// The live task already holding `identity`, oldest first, in any
/// queue.
pub(crate) fn live_identity_holder(
    t: &TursoBackend,
    identity: &WorkIdentity,
    kind: &str,
) -> Result<Option<Value>, TErr> {
    let rows = if kind == "prompt" {
        t.query_objs(
            &format!(
                "SELECT {TASK_COLS} FROM tasks \
                 WHERE status IN ('queued', 'claimed', 'parked', 'handed_off', 'blocked') \
                   AND json_extract(payload, '$.kind') = ? \
                 ORDER BY id ASC"
            ),
            &[Arg::Text(kind.to_string())],
            &TASK_JSON_COLS,
        )?
    } else {
        t.query_objs(
            &format!(
                "SELECT {TASK_COLS} FROM tasks \
                 WHERE status IN ('queued', 'claimed', 'parked', 'handed_off', 'blocked') \
                   AND json_extract(payload, '$.kind') = ? \
                   AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ? \
                 ORDER BY id ASC"
            ),
            &[Arg::Text(kind.to_string()), Arg::Int(identity.issue)],
            &TASK_JSON_COLS,
        )?
    };
    Ok(rows.into_iter().find(|row| holds_identity(row, identity)))
}

/// The completed task already holding `identity`, oldest first, in
/// any queue (#13091): the same match as live_identity_holder over
/// done rows. Only completion absorbs — failed and cancelled rows
/// are re-queueable by design (#10627).
pub(crate) fn done_identity_holder(
    t: &TursoBackend,
    identity: &WorkIdentity,
    kind: &str,
) -> Result<Option<Value>, TErr> {
    let rows = if kind == "prompt" {
        t.query_objs(
            &format!(
                "SELECT {TASK_COLS} FROM tasks \
                 WHERE status = 'done' \
                   AND json_extract(payload, '$.kind') = ? \
                 ORDER BY id ASC"
            ),
            &[Arg::Text(kind.to_string())],
            &TASK_JSON_COLS,
        )?
    } else {
        t.query_objs(
            &format!(
                "SELECT {TASK_COLS} FROM tasks \
                 WHERE status = 'done' \
                   AND json_extract(payload, '$.kind') = ? \
                   AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ? \
                 ORDER BY id ASC"
            ),
            &[Arg::Text(kind.to_string()), Arg::Int(identity.issue)],
            &TASK_JSON_COLS,
        )?
    };
    Ok(rows
        .into_iter()
        .find(|row| holds_completed_identity(row, identity)))
}

/// #16007: whether a completed triage's issue is left uncovered by
/// its fix follow-ons. Recovery is needed when no follow-on is live
/// (queued/claimed/parked/handed_off/blocked) and none completed:
/// every follow-on failed or was cancelled, or none ever landed.
/// A live follow-on still covers the issue (the completed triage
/// absorbs the re-filing), and a completed follow-on is the work
/// actually done (it absorbs too).
pub fn triage_recovery_needed(follow_on_statuses: &[&str]) -> bool {
    !follow_on_statuses
        .iter()
        .any(|s| LIVE_STATUSES.contains(s) || *s == "done")
}

/// The statuses of every fix follow-on of the triage work named by
/// `identity` (#16007): rows in any queue whose work identity is the
/// fix-family identity for the same repo and issue — the follow-on
/// a triage completion files (`small_fix`, #7016). Status strings
/// only; `triage_recovery_needed` decides what they mean.
pub(crate) fn triage_follow_on_statuses(
    t: &TursoBackend,
    identity: &WorkIdentity,
) -> Result<Vec<String>, TErr> {
    let fix_identity = WorkIdentity {
        repo: identity.repo.clone(),
        issue: identity.issue,
        family: "fix".to_string(),
        prs: Vec::new(),
    };
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks \
             WHERE json_extract(payload, '$.kind') = 'small_fix' \
               AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ? \
             ORDER BY id ASC"
        ),
        &[Arg::Int(identity.issue)],
        &TASK_JSON_COLS,
    )?;
    Ok(rows
        .iter()
        .filter(|row| work_identity(row).as_ref() == Some(&fix_identity))
        .filter_map(|row| {
            row.get("status")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect())
}

/// The machine-readable report when an enqueue's dedupe key already
/// names the returned row (#10627, fleet-monitor ask 2026-11-25): the
/// filing was absorbed and nothing was created, and the reply must
/// say so in a field of its own — a bare row reads as a successful
/// filing, and a completed row returned this way left `queue/queued`
/// issues holding no live task and no signal (the monitor sampled
/// tasks 18037, 22316, 22051, 23610 returned for fresh filings).
/// `terminal` separates the completed predecessor (the caller
/// suffixes the key and re-files in the same pass) from a live hit
/// (the caller attaches follow-ups to the holder).
pub fn dedupe_match_detail(row: &Value, terminal: bool) -> Value {
    let (rule, hint) = if terminal {
        (
            "dedupe_key_terminal",
            "the dedupe key already belongs to a completed task; no new task was \
             filed — suffix the dedupe key to file this work again, or file with \
             payload.dedupe_override=\"<reason>\" for a deliberate parallel",
        )
    } else {
        (
            "dedupe_key_live",
            "the dedupe key already has a live task; no new task was filed — \
             attach follow-ups to the existing task, or file with \
             payload.dedupe_override=\"<reason>\" for a deliberate parallel",
        )
    };
    json!({
        "rule": rule,
        "task_id": row.get("id").cloned().unwrap_or(Value::Null),
        "status": row.get("status").cloned().unwrap_or(Value::Null),
        "queue": row.get("queue").cloned().unwrap_or(Value::Null),
        "dedupe_key": row.get("dedupe_key").cloned().unwrap_or(Value::Null),
        "hint": hint,
    })
}

/// The machine-readable collision a deduped enqueue reports.
pub fn collision_detail(holder: &Value, identity: &WorkIdentity) -> Value {
    json!({
        "rule": "open_work_identity",
        "identity": identity.key(),
        "task_id": holder.get("id").cloned().unwrap_or(Value::Null),
        "status": holder.get("status").cloned().unwrap_or(Value::Null),
        "queue": holder.get("queue").cloned().unwrap_or(Value::Null),
        "dedupe_key": holder.get("dedupe_key").cloned().unwrap_or(Value::Null),
        "hint": "this work already has a live task; attach follow-ups to it, \
                 or file with payload.dedupe_override=\"<reason>\" for a deliberate parallel",
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn task(status: &str, payload: Value) -> Value {
        json!({"id": 3, "queue": "repo-health-code-pilot", "status": status, "payload": payload})
    }

    fn fix(n: i64) -> Value {
        json!({"kind": "small_fix", "repo": "developing-today/code", "issue": {"number": n}})
    }

    #[test]
    fn identity_is_repo_issue_and_family() {
        let id = work_identity(&task("queued", fix(5))).unwrap();
        assert_eq!(id.key(), "developing-today/code#5:fix");
        // A payload with no repo resolves through the queue name; a
        // queue that names no owner/repo gives no identity (fail open).
        let bare = json!({"kind": "triage", "issue": {"number": 5}});
        assert!(work_identity(&task("queued", bare)).is_none());
    }

    #[test]
    fn prompt_identity_uses_submission_id_not_source_issue() {
        let prompt = |id: &str| {
            json!({
                "kind": "prompt", "repo": "developing-today/code",
                "issue": {"number": 10032}, "prompt": {"id": id}
            })
        };
        let first = work_identity(&task(
            "queued",
            prompt("prompt:00000000-0000-4000-8000-000000000001"),
        ))
        .unwrap();
        let repeated = work_identity(&task(
            "queued",
            prompt("prompt:00000000-0000-4000-8000-000000000001"),
        ))
        .unwrap();
        let distinct = work_identity(&task(
            "queued",
            prompt("prompt:00000000-0000-4000-8000-000000000002"),
        ))
        .unwrap();
        assert_eq!(
            first.key(),
            "developing-today/code#prompt:00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(first, repeated, "same submission ID must dedupe");
        assert_ne!(
            first, distinct,
            "different submission IDs remain distinct on the same issue"
        );
    }

    #[test]
    fn recurring_kinds_exempt_and_issueless_take_no_part() {
        for kind in ["ci_investigation", "second_pass", "feature_slice"] {
            let p = json!({"kind": kind, "repo": "developing-today/code", "issue": {"number": 5}});
            assert!(work_identity(&task("queued", p)).is_none(), "{kind}");
        }
        let mut p = fix(5);
        p["dedupe_exempt"] = json!(true);
        assert!(work_identity(&task("queued", p)).is_none());
        let p = json!({"kind": "small_fix", "repo": "developing-today/code"});
        assert!(work_identity(&task("queued", p)).is_none());
    }

    #[test]
    fn shepherds_differ_per_pr_set() {
        let a = json!({"kind": "pr_shepherd", "repo": "developing-today/code",
                       "issue": {"number": 7297}, "pr": {"number": 12}});
        let b = json!({"kind": "pr_shepherd", "repo": "developing-today/code",
                       "issue": {"number": 7297}, "prs": ["https://github.com/developing-today/code/pull/12"]});
        let c = json!({"kind": "pr_shepherd", "repo": "developing-today/code",
                       "issue": {"number": 7297}, "pr": {"number": 13}});
        let ia = work_identity(&task("queued", a)).unwrap();
        assert_eq!(ia.key(), "developing-today/code#7297:pr_shepherd:pr12");
        assert_eq!(Some(ia.clone()), work_identity(&task("queued", b)));
        assert_ne!(Some(ia), work_identity(&task("queued", c)));
    }

    #[test]
    fn only_live_valid_rows_hold_an_identity() {
        let id = work_identity(&task("queued", fix(5))).unwrap();
        // A full, valid packet on a live row holds it.
        let valid = json!({
            "id": 9, "queue": "repo-health-code-pilot", "status": "parked",
            "title": "t", "priority": 30,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:5",
            "payload": {
                "schema": "repo-task/v1", "kind": "small_fix", "repo": "developing-today/code",
                "problem": "p", "priority_reason": "r", "evidence": ["e"], "acceptance": ["a"],
                "tests": ["t"], "context_refs": [], "scope": {"include": ["x"], "exclude": ["y"]},
                "risk": {"level": "low", "flags": []}, "timebox_minutes": 45,
                "issue": {"number": 5},
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
            }
        });
        assert!(
            holds_identity(&valid, &id),
            "{:?}",
            claim_guard_reason(&valid)
        );
        for status in ["done", "failed", "cancelled"] {
            let mut r = valid.clone();
            r["status"] = json!(status);
            assert!(!holds_identity(&r, &id), "{status}");
        }
        // A quarantined (invalid) blocked row never absorbs a filing.
        let quarantined = task("blocked", fix(5));
        assert!(!holds_identity(&quarantined, &id));
        // Another issue or family does not collide.
        let mut other = valid.clone();
        other["payload"]["issue"]["number"] = json!(6);
        assert!(!holds_identity(&other, &id));
        let mut triage = valid.clone();
        triage["payload"]["kind"] = json!("triage");
        assert!(!holds_identity(&triage, &id));
    }

    #[test]
    fn override_needs_a_reason() {
        assert_eq!(
            dedupe_override(&json!({"payload": {"dedupe_override": " parallel spike "}})),
            Some("parallel spike".to_string())
        );
        assert_eq!(
            dedupe_override(&json!({"payload": {"dedupe_override": ""}})),
            None
        );
        assert_eq!(
            dedupe_override(&json!({"payload": {"dedupe_override": true}})),
            None
        );
        assert_eq!(dedupe_override(&json!({"payload": {}})), None);
    }

    #[test]
    fn collision_is_machine_readable() {
        let id = work_identity(&task("queued", fix(5))).unwrap();
        let holder = json!({"id": 41, "status": "claimed", "queue": "q", "dedupe_key": "k"});
        let c = collision_detail(&holder, &id);
        assert_eq!(c["task_id"], 41);
        assert_eq!(c["status"], "claimed");
        assert_eq!(c["identity"], "developing-today/code#5:fix");
        assert_eq!(c["rule"], "open_work_identity");
    }

    #[test]
    fn dedupe_match_is_machine_readable() {
        let done = json!({"id": 7, "status": "done", "queue": "q", "dedupe_key": "k"});
        let m = dedupe_match_detail(&done, true);
        assert_eq!(m["rule"], "dedupe_key_terminal");
        assert_eq!(m["task_id"], 7);
        assert_eq!(m["status"], "done");
        assert_eq!(m["dedupe_key"], "k");
        let live = json!({"id": 8, "status": "claimed", "queue": "q", "dedupe_key": "k"});
        let m = dedupe_match_detail(&live, false);
        assert_eq!(m["rule"], "dedupe_key_live");
        assert_eq!(m["task_id"], 8);
        assert_eq!(m["status"], "claimed");
    }

    #[test]
    fn triage_recovery_only_when_no_follow_on_covers_the_issue_16007() {
        // #16007 oracle: a completed triage task with an open issue
        // and a failed or cancelled fix follow-on is re-queueable —
        // the recovery filing must not be absorbed.
        assert!(triage_recovery_needed(&["failed"]));
        assert!(triage_recovery_needed(&["cancelled"]));
        assert!(triage_recovery_needed(&["failed", "cancelled"]));
        // No follow-on ever landed: nothing covers the issue either.
        assert!(triage_recovery_needed(&[]));
        // A live follow-on covers the issue: no recovery triage.
        for status in ["queued", "claimed", "parked", "handed_off", "blocked"] {
            assert!(!triage_recovery_needed(&[status]), "{status}");
            assert!(!triage_recovery_needed(&["failed", status]), "{status}");
        }
        // A completed follow-on is the work done: no recovery triage,
        // even alongside an earlier failed attempt.
        assert!(!triage_recovery_needed(&["done"]));
        assert!(!triage_recovery_needed(&["failed", "done"]));
        assert!(!triage_recovery_needed(&["cancelled", "done"]));
    }

    #[test]
    fn triage_recovery_is_checked_in_both_done_absorption_paths_16007() {
        // #16007 wiring oracle: BOTH completed-work absorptions in
        // verb_enqueue — the same-dedupe-key terminal hit and the
        // done work-identity holder — consult the triage recovery
        // check before returning the completed triage row, so the
        // same-key and the same-work-identity re-filings both file a
        // fresh claimable triage task in the recovery state.
        let src = include_str!("claim.rs");
        let start = src.find("fn verb_enqueue(").unwrap();
        let body = &src[start..];
        let key_branch = body
            .find("dedupe_match_detail(row, true)")
            .unwrap_or(usize::MAX);
        assert_eq!(
            body.matches("triage_recovery_filing(").count(),
            2,
            "both done-absorption paths must check triage recovery"
        );
        let first_check = body.find("triage_recovery_filing(").unwrap();
        let done_identity = body.find("done_identity_holder(").unwrap();
        assert!(first_check < key_branch || first_check < done_identity);
        // Completed non-triage work still absorbs: both terminal
        // reports remain in the enqueue body.
        assert!(body.contains("dedupe_match_detail(row, true)"));
        assert!(body.contains("done_work_identity"));
    }

    #[test]
    fn enqueue_checks_identity_after_the_dedupe_key_and_before_insert() {
        let src = include_str!("claim.rs");
        let start = src.find("fn verb_enqueue(").unwrap();
        let body = &src[start..];
        let key = body.find("dedupe_partition(&existing)").unwrap();
        let ident = body.find("live_identity_holder(").unwrap();
        let insert = body.find("INSERT INTO tasks").unwrap();
        assert!(key < ident && ident < insert);
    }
}
