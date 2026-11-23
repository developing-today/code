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
//! follow-up onto it. Terminal tasks never block a new filing
//! (re-queue after failure is unaffected; a fresh already-done
//! verdict is the claim-time staleness check's job, #10626).
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
    /// The shepherded PRs, sorted; empty for other families.
    pub prs: Vec<i64>,
}

impl WorkIdentity {
    pub fn key(&self) -> String {
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
    let issue = payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64())
        .filter(|n| *n > 0)?;
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

/// Whether a live row holds `identity` and may absorb a new filing:
/// same identity, and not a quarantined (invalid) row.
pub fn holds_identity(row: &Value, identity: &WorkIdentity) -> bool {
    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if !LIVE_STATUSES.contains(&status) {
        return false;
    }
    if work_identity(row).as_ref() != Some(identity) {
        return false;
    }
    claim_guard_reason(row).is_none()
}

/// The live task already holding `identity`, oldest first, in any
/// queue.
pub(crate) fn live_identity_holder(
    t: &TursoBackend,
    identity: &WorkIdentity,
    kind: &str,
) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks \
             WHERE status IN ('queued', 'claimed', 'parked', 'handed_off', 'blocked') \
               AND json_extract(payload, '$.kind') = ? \
               AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ? \
             ORDER BY id ASC"
        ),
        &[Arg::Text(kind.to_string()), Arg::Int(identity.issue)],
        &TASK_JSON_COLS,
    )?;
    Ok(rows.into_iter().find(|row| holds_identity(row, identity)))
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
