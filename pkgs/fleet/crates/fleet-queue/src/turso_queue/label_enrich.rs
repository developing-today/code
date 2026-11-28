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
