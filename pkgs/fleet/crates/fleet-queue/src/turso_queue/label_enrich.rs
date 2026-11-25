//! turso_queue::label_enrich — enqueue-time label carriage from the
//! GitHub mirror (#12744).
//!
//! Slice 1 of #7030 (PR #12464) made the enqueue row's stored
//! priority derive from the payload's `labels` array (expedite ⇒
//! 15, next-up ⇒ 25; see validate::label_band_priority). The
//! derivation was inert in production: no packet producer writes
//! `payload.labels` — of the 70 tasks created after the slice
//! merged, 0 carried a labels array, and both live tasks for the
//! one open expedite issue (#12228) reached the store at their
//! stated 30, banded only by the manual re-band recorded on #7030.
//!
//! The durable fix is broker-side, at the single source of truth:
//! when an enqueue (or the broker's own triage follow-on) builds a
//! row for a payload that identifies a GitHub issue but carries no
//! labels array of its own, the broker fills `labels` from
//! `github_mirror_issues` — the GitHub mirror the github-ci Worker
//! maintains in this same store (pkgs/github-ci/worker/mirror.sql),
//! whose `labels` column is the issue's GitHub label names as a
//! JSON array, the exact shape every payload.labels reader (the
//! band selection, the claim filter's label field) already reads.
//! The bands then fire on every packet regardless of producer
//! discipline, and the stored payload carries the labels the claim
//! filter matches on.
//!
//! Semantics, deliberately narrow:
//! - A labels array already on the payload stands untouched: it is
//!   the producer's own enqueue-time read of the issue's GitHub
//!   labels. The mirror fills only an absent (or non-array) key.
//! - The mirror is a read model and never gates work (#8575): a
//!   missing table or a failed lookup is reported to the caller,
//!   which proceeds unenriched — the packet's stated priority and
//!   payload stand exactly as before this change.
//! - An issue with no mirror row, or a mirror row with no labels,
//!   leaves the payload untouched: absence already reads as "no
//!   band labels" to every consumer, so no empty array is written.
//! - Identity comes from the payload itself (`repo` +
//!   `issue.number`, the repo-task/v1 shape), with the repo-health
//!   dedupe key as the repo fallback for packets that name the
//!   issue only there. No identity, no lookup.

use super::*;

/// The GitHub issue a payload identifies, as (repo, number). The
/// repo is the payload's own `repo` (or `issue.repo`); for the
/// repo-health packet family the dedupe key names it too
/// (`repo-health:v1:<kind>:<repo>:issue:<n>`, see
/// validate::DEDUPE_PATTERNS_DOC), and a packet that carries the
/// repo only there still identifies its issue. A repo without a
/// '/' can never key the mirror (its keys are full `owner/name`),
/// so it identifies nothing here.
pub(crate) fn payload_issue_ref(
    payload: &Value,
    dedupe_key: Option<&str>,
) -> Option<(String, i64)> {
    let number = payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64())
        .filter(|n| *n > 0)?;
    let repo = payload
        .get("repo")
        .and_then(|v| v.as_str())
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("issue")
                .and_then(|i| i.get("repo"))
                .and_then(|v| v.as_str())
                .filter(|r| !r.is_empty())
                .map(str::to_string)
        })
        .or_else(|| dedupe_key.and_then(repo_from_dedupe_key))?;
    if !repo.contains('/') {
        return None;
    }
    Some((repo, number))
}

/// The repo a repo-health dedupe key names: the segment between
/// the kind segment and the `:issue:` marker. Anything that does
/// not parse to that shape names nothing.
fn repo_from_dedupe_key(dedupe_key: &str) -> Option<String> {
    let rest = dedupe_key.strip_prefix("repo-health:v1:")?;
    let (_, tail) = rest.split_once(':')?;
    let repo = tail.split(":issue:").next()?;
    if repo.is_empty() || repo == tail {
        return None;
    }
    Some(repo.to_string())
}

/// The labels the GitHub mirror holds for one issue: Ok(Some) with
/// the label names when a mirror row exists (possibly an empty
/// vec — the issue is mirrored and carries no labels), Ok(None)
/// when no row exists. The mirror's `labels` column is a JSON
/// array of names; non-string entries, if a future mirror shape
/// ever wrote them, are dropped rather than forwarded.
pub(crate) fn mirror_issue_labels(
    t: &TursoBackend,
    repo: &str,
    number: i64,
) -> Result<Option<Vec<String>>, TErr> {
    let rows = t.query_objs(
        "SELECT labels FROM github_mirror_issues WHERE repo = ? AND number = ?",
        &[Arg::Text(repo.to_string()), Arg::Int(number)],
        &[],
    )?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let raw = row.get("labels").cloned().unwrap_or(Value::Null);
    let parsed: Vec<String> = match &raw {
        Value::String(s) => serde_json::from_str::<Value>(s)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|l| l.as_str().map(str::to_string))
            .collect(),
        Value::Array(a) => a
            .iter()
            .filter_map(|l| l.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    Ok(Some(parsed))
}

/// Fill `payload`'s `labels` from the mirror when the payload
/// identifies an issue and carries no labels array of its own.
/// Ok(true) when labels were written; Ok(false) when the payload
/// stands as it was (already labelled, no identity, no mirror row,
/// or a mirror row with no labels). Err only when the mirror
/// lookup itself failed — the caller decides what that costs (the
/// enqueue path warns and proceeds; the mirror never gates work).
pub(crate) fn enrich_payload_labels(
    t: &TursoBackend,
    payload: &mut Value,
    dedupe_key: Option<&str>,
) -> Result<bool, TErr> {
    if payload.get("labels").map(|l| l.is_array()) == Some(true) {
        return Ok(false);
    }
    let Some((repo, number)) = payload_issue_ref(payload, dedupe_key) else {
        return Ok(false);
    };
    let Some(labels) = mirror_issue_labels(t, &repo, number)? else {
        return Ok(false);
    };
    if labels.is_empty() {
        return Ok(false);
    }
    let Some(obj) = payload.as_object_mut() else {
        return Ok(false);
    };
    obj.insert("labels".to_string(), json!(labels));
    Ok(true)
}

/// The stored priority for a broker-built row that inherits one
/// (the triage follow-on): Drew's label band when the (possibly
/// just-enriched) payload carries a band label — the label is the
/// signal, the inherited number is earlier machinery's output,
/// the same rule task_from_packet applies at enqueue — and the
/// inherited priority otherwise.
pub(crate) fn band_or_inherited(payload: &Value, inherited: Option<i64>) -> Option<i64> {
    crate::validate::label_band_priority(payload).or(inherited)
}

// ---------------------------------------------------------------------------
// #14170 Q5 — the task-row label cache (`tasks.labels`)
// ---------------------------------------------------------------------------
//
// The mirror stays the source of truth for an issue's current GitHub
// labels; `tasks.labels` is a cache for the readers that cannot join
// per row (list filtering, held-set audits). It holds the mirror's
// exact shape (a JSON array of names) and `labels_synced_at` stamps
// the moment it was copied, so a reader sees staleness instead of
// trusting silently. A task whose issue has no mirror row keeps its
// `[]` and a NULL stamp, which reads honestly as "never synced".
//
// Refresh moments: every lifecycle transition (core::event, which
// covers enqueue and claim), plus the hourly actionable-only sweep
// that rides /sweep, plus /labels-sync on demand (`all: true` is
// the one-off backfill over every task). Every refresh is fail-open:
// the mirror is a read model and never gates work (#8575).

/// Statuses the hourly sweep keeps fresh: the items a reader is about
/// to act on. Terminal rows are refreshed only by `/labels-sync` with
/// `all: true`.
pub(crate) const LABEL_SYNC_STATUSES: [&str; 4] = ["queued", "claimed", "parked", "blocked"];

/// How long the sweep waits between label passes (seconds).
pub(crate) const LABEL_SYNC_INTERVAL_SECONDS: i64 = 3600;

/// One task's cache refresh from the mirror. Binds: repo, number,
/// synced_at, task id, repo, number. Writes nothing (and leaves the
/// stamp alone) when the mirror has no row for the issue; a mirror
/// row whose labels are absent or not a JSON array caches `[]`.
pub(crate) const LABELS_REFRESH_SQL: &str = "UPDATE tasks SET \
    labels = COALESCE((SELECT CASE WHEN json_valid(m.labels) AND json_type(m.labels) = 'array' \
        THEN m.labels ELSE '[]' END FROM github_mirror_issues m WHERE m.repo = ? AND m.number = ?), '[]'), \
    labels_synced_at = ? \
    WHERE id = ? AND EXISTS (SELECT 1 FROM github_mirror_issues m WHERE m.repo = ? AND m.number = ?)";

/// The bound arguments for LABELS_REFRESH_SQL.
pub(crate) fn labels_refresh_args(
    task_id: i64,
    repo: &str,
    number: i64,
    synced_at: &str,
) -> Vec<Arg> {
    vec![
        Arg::Text(repo.to_string()),
        Arg::Int(number),
        Arg::Text(synced_at.to_string()),
        Arg::Int(task_id),
        Arg::Text(repo.to_string()),
        Arg::Int(number),
    ]
}

/// The issue ref of a stored task row (payload + dedupe_key), by the
/// same rule enqueue enrichment uses.
fn row_issue_ref(row: &Value) -> Option<(String, i64)> {
    let payload = match row.get("payload") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    payload_issue_ref(&payload, row.get("dedupe_key").and_then(|v| v.as_str()))
}

/// Refresh one task's label cache from the mirror. Ok(false) when the
/// task has no issue ref (nothing to look up); Ok(true) when the
/// refresh ran (the row is stamped only if the mirror has the issue).
/// Err when the store or the mirror table could not be read — the
/// caller ignores it (fail-open).
pub(crate) fn refresh_task_labels(t: &TursoBackend, task_id: i64) -> Result<bool, TErr> {
    let rows = t.query_objs(
        "SELECT payload, dedupe_key FROM tasks WHERE id = ?",
        &[Arg::Int(task_id)],
        &[],
    )?;
    let Some((repo, number)) = rows.first().and_then(row_issue_ref) else {
        return Ok(false);
    };
    t.exec(
        LABELS_REFRESH_SQL,
        &labels_refresh_args(task_id, &repo, number, &now_iso()),
    )?;
    Ok(true)
}

/// Refresh the label cache for many tasks: every actionable task, or
/// every task with `all`. Rows are read in id pages and refreshed in
/// one pipeline per page. Reports what it scanned, how many rows had
/// an issue ref, and how many the mirror actually stamped.
pub(crate) fn sync_task_labels(t: &TursoBackend, all: bool, page_size: i64) -> Result<Value, TErr> {
    let page_size = page_size.clamp(1, 1000);
    let synced_at = now_iso();
    let status_cond = if all {
        String::new()
    } else {
        let list = LABEL_SYNC_STATUSES
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" AND status IN ({list})")
    };
    let (mut scanned, mut with_ref, mut after) = (0i64, 0i64, 0i64);
    loop {
        let rows = t.query_objs(
            &format!(
                "SELECT id, payload, dedupe_key FROM tasks WHERE id > ?{status_cond} \
                 ORDER BY id LIMIT {page_size}"
            ),
            &[Arg::Int(after)],
            &[],
        )?;
        let Some(last) = rows
            .last()
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_i64())
        else {
            break;
        };
        after = last;
        scanned += rows.len() as i64;
        let stmts: Vec<(&str, Vec<Arg>)> = rows
            .iter()
            .filter_map(|r| {
                let id = r.get("id").and_then(|v| v.as_i64())?;
                let (repo, number) = row_issue_ref(r)?;
                Some((
                    LABELS_REFRESH_SQL,
                    labels_refresh_args(id, &repo, number, &synced_at),
                ))
            })
            .collect();
        with_ref += stmts.len() as i64;
        if !stmts.is_empty() {
            t.exec_batch(&stmts)?;
        }
        if (rows.len() as i64) < page_size {
            break;
        }
    }
    let stamped = t
        .query_objs(
            "SELECT COUNT(*) AS n FROM tasks WHERE labels_synced_at = ?",
            &[Arg::Text(synced_at.clone())],
            &[],
        )?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Ok(json!({
        "scope": if all { "all" } else { "actionable" },
        "scanned": scanned,
        "with_issue_ref": with_ref,
        "stamped": stamped,
        "synced_at": synced_at,
    }))
}

/// The sweep's hourly label pass: runs when the last pass is older
/// than LABEL_SYNC_INTERVAL_SECONDS (or never ran), records the pass
/// in checkpoints, and never fails the sweep — its error is reported
/// inside its own summary. Ok(None) when the interval has not passed.
pub(crate) fn sweep_task_labels(t: &TursoBackend, force: bool) -> Option<Value> {
    let work = || -> Result<Option<Value>, TErr> {
        if !force {
            let last = t
                .query_objs(
                    "SELECT updated_at FROM checkpoints \
                     WHERE worker_id = 'fleet-sweeper' AND key = 'labels_sync'",
                    &[],
                    &[],
                )?
                .first()
                .and_then(|r| r.get("updated_at"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let due_before = iso(now_unix() - LABEL_SYNC_INTERVAL_SECONDS);
            if last.is_some_and(|l| l > due_before) {
                return Ok(None);
            }
        }
        let summary = sync_task_labels(t, false, 500)?;
        t.exec(
            "INSERT INTO checkpoints (worker_id, key, value, updated_at) \
             VALUES ('fleet-sweeper', 'labels_sync', ?, ?) \
             ON CONFLICT(worker_id, key) DO UPDATE SET \
               value = excluded.value, updated_at = excluded.updated_at",
            &[Arg::Text(summary.to_string()), Arg::Text(now_iso())],
        )?;
        Ok(Some(summary))
    };
    match work() {
        Ok(v) => v,
        Err(e) => Some(json!({"error": e})),
    }
}

/// `/labels-sync` (#14170 Q5): refresh the task label cache now.
/// `all: true` covers every task (the one-off backfill); otherwise
/// only actionable statuses. `page_size` bounds each read page.
pub(crate) fn verb_labels_sync(t: &TursoBackend, req: &Value) -> Value {
    let all = req_bool(req, "all").unwrap_or(false);
    let page = req_i64(req, "page_size").unwrap_or(500);
    match sync_task_labels(t, all, page) {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// The list filter for one `--label` value: an exact label name, or
/// a prefix glob ending in `*` (`user/*`). Returns the SQL condition
/// and its bound argument.
pub(crate) fn label_filter_cond(label: &str) -> (&'static str, Arg) {
    match label.strip_suffix('*') {
        Some(prefix) => {
            let escaped = prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            (
                " AND EXISTS (SELECT 1 FROM json_each(tasks.labels) \
                 WHERE json_each.value LIKE ? ESCAPE '\\')",
                Arg::Text(format!("{escaped}%")),
            )
        }
        None => (
            " AND EXISTS (SELECT 1 FROM json_each(tasks.labels) WHERE json_each.value = ?)",
            Arg::Text(label.to_string()),
        ),
    }
}
