//! Q6 normalized task links and dependency storage helpers (#14170).

use super::*;

const LINK_KINDS: [&str; 8] = [
    "implements",
    "pr_for",
    "parent",
    "child",
    "split_from",
    "supersedes",
    "duplicate_of",
    "related",
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkRef {
    other_kind: &'static str,
    repo: Option<String>,
    number: Option<i64>,
    other_task_id: Option<i64>,
    link_kind: String,
    link_key: String,
}

impl LinkRef {
    fn issue_or_pr(kind: &'static str, repo: String, number: i64, link_kind: &str) -> Self {
        Self {
            other_kind: kind,
            link_key: format!("{kind}:{repo}#{number}"),
            repo: Some(repo),
            number: Some(number),
            other_task_id: None,
            link_kind: link_kind.to_owned(),
        }
    }

    fn task(task_id: i64, link_kind: &str) -> Self {
        Self {
            other_kind: "task",
            repo: None,
            number: None,
            other_task_id: Some(task_id),
            link_key: format!("task:{task_id}"),
            link_kind: link_kind.to_owned(),
        }
    }
}

fn link_kind(value: &str) -> Result<String, String> {
    if LINK_KINDS.contains(&value) {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "link: kind {value:?} is not one of {}",
            LINK_KINDS.join(", ")
        ))
    }
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty())
}

fn request_link(req: &Value) -> Result<LinkRef, String> {
    let kind = link_kind(req_str(req, "link_kind").unwrap_or(""))?;
    let target_count = ["other_issue", "other_pr", "other_task_id"]
        .iter()
        .filter(|key| req.get(**key).is_some_and(|v| !v.is_null()))
        .count();
    if target_count != 1 {
        return Err(
            "link: exactly one of other_issue, other_pr, or other_task_id is required".into(),
        );
    }
    if let Some(target) = req.get("other_issue").filter(|v| !v.is_null()) {
        let repo = target
            .get("repo")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let number = target.get("number").and_then(Value::as_i64).unwrap_or(0);
        if !valid_repo(repo) || number <= 0 {
            return Err("link: issue target requires owner/repo and a positive number".into());
        }
        if kind == "pr_for" {
            return Err("link: pr_for requires a pull-request target".into());
        }
        return Ok(LinkRef::issue_or_pr(
            "issue",
            repo.to_owned(),
            number,
            &kind,
        ));
    }
    if let Some(target) = req.get("other_pr").filter(|v| !v.is_null()) {
        let repo = target
            .get("repo")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let number = target.get("number").and_then(Value::as_i64).unwrap_or(0);
        if !valid_repo(repo) || number <= 0 {
            return Err(
                "link: pull-request target requires owner/repo and a positive number".into(),
            );
        }
        if kind == "implements" {
            return Err("link: implements requires an issue target".into());
        }
        return Ok(LinkRef::issue_or_pr("pr", repo.to_owned(), number, &kind));
    }
    let task_id = req
        .get("other_task_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if task_id <= 0 {
        return Err("link: task target requires a positive task id".into());
    }
    Ok(LinkRef::task(task_id, &kind))
}

fn parse_json(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}

fn dedupe_issue_ref(key: &str) -> Option<(String, i64)> {
    let rest = key.strip_prefix("repo-health:v1:")?;
    let (_, tail) = rest.split_once(':')?;
    let (repo, number) = tail.split_once(":issue:")?;
    let number = number.split(':').next()?.parse::<i64>().ok()?;
    (valid_repo(repo) && number > 0).then(|| (repo.to_owned(), number))
}

fn dedupe_pr_refs(key: &str) -> Vec<(String, i64)> {
    let Some(rest) = key.strip_prefix("repo-health:v1:") else {
        return Vec::new();
    };
    let Some((family, tail)) = rest.split_once(':') else {
        return Vec::new();
    };
    match family {
        "pr" => {
            let Some((repo, number)) = tail.split_once(":pr:") else {
                return Vec::new();
            };
            let number = number.split(':').next().and_then(|n| n.parse::<i64>().ok());
            number
                .filter(|n| valid_repo(repo) && *n > 0)
                .map(|n| vec![(repo.to_owned(), n)])
                .unwrap_or_default()
        }
        "pr-set" => {
            let Some((repo, numbers)) = tail.split_once(':') else {
                return Vec::new();
            };
            if !valid_repo(repo) {
                return Vec::new();
            }
            let mut refs: Vec<(String, i64)> = numbers
                .split('-')
                .filter_map(|number| number.parse::<i64>().ok())
                .filter(|number| *number > 0)
                .map(|number| (repo.to_owned(), number))
                .collect();
            refs.sort();
            refs.dedup();
            refs
        }
        _ => Vec::new(),
    }
}

fn parse_pull_url(url: &str) -> (Option<String>, Option<i64>) {
    let Some(rest) = url.split("github.com/").nth(1) else {
        return (None, None);
    };
    let Some((repo, tail)) = rest.split_once("/pull/") else {
        return (None, None);
    };
    if !valid_repo(repo) {
        return (None, None);
    }
    let number = tail
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| *n > 0);
    (Some(repo.to_owned()), number)
}

fn push_pr_target(value: &Value, fallback_repo: Option<&str>, out: &mut Vec<LinkRef>) {
    let (repo, number) = match value {
        Value::Number(n) => (fallback_repo.map(str::to_owned), n.as_i64()),
        Value::String(s) => {
            let (repo, number) = parse_pull_url(s);
            (repo.or_else(|| fallback_repo.map(str::to_owned)), number)
        }
        Value::Object(o) => {
            let url = o.get("url").and_then(Value::as_str).unwrap_or("");
            let repo = o
                .get("repo")
                .and_then(Value::as_str)
                .filter(|r| valid_repo(r))
                .map(str::to_owned)
                .or_else(|| parse_pull_url(url).0)
                .or_else(|| fallback_repo.map(str::to_owned));
            let number = o
                .get("number")
                .and_then(Value::as_i64)
                .or_else(|| parse_pull_url(url).1);
            (repo, number)
        }
        _ => (None, None),
    };
    if let (Some(repo), Some(number)) = (repo, number.filter(|n| *n > 0)) {
        if valid_repo(&repo) {
            out.push(LinkRef::issue_or_pr("pr", repo, number, "pr_for"));
        }
    }
}

fn task_link_refs(row: &Value) -> Vec<LinkRef> {
    let payload = parse_json(row.get("payload"));
    let result = parse_json(row.get("result"));
    let repo = payload
        .get("repo")
        .and_then(Value::as_str)
        .filter(|r| valid_repo(r))
        .or_else(|| {
            payload
                .get("issue")
                .and_then(|i| i.get("repo"))
                .and_then(Value::as_str)
                .filter(|r| valid_repo(r))
        })
        .map(str::to_owned);
    let dedupe_key = row.get("dedupe_key").and_then(Value::as_str);
    let mut links = Vec::new();
    if let Some((issue_repo, number)) = super::payload_issue_ref(&payload, dedupe_key)
        .or_else(|| dedupe_key.and_then(dedupe_issue_ref))
    {
        links.push(LinkRef::issue_or_pr(
            "issue",
            issue_repo,
            number,
            "implements",
        ));
    }
    if let Some(key) = dedupe_key {
        for (pr_repo, number) in dedupe_pr_refs(key) {
            links.push(LinkRef::issue_or_pr("pr", pr_repo, number, "pr_for"));
        }
    }
    for pr in [payload.get("pr"), result.get("pr"), result.get("pr_url")]
        .into_iter()
        .flatten()
    {
        push_pr_target(pr, repo.as_deref(), &mut links);
    }
    for prs in [
        payload.get("prs").and_then(Value::as_array),
        result.get("prs").and_then(Value::as_array),
        result
            .get("disposition")
            .and_then(|d| d.get("pulls"))
            .and_then(Value::as_array),
    ]
    .into_iter()
    .flatten()
    {
        for pr in prs {
            push_pr_target(pr, repo.as_deref(), &mut links);
        }
    }
    for parent in [
        payload.get("parent_issue"),
        payload.get("slice").and_then(|s| s.get("parent_issue")),
        payload.get("set").and_then(|s| s.get("origin_issue")),
    ]
    .into_iter()
    .flatten()
    {
        let (parent_repo, number) = match parent {
            Value::Number(n) => (repo.clone(), n.as_i64()),
            Value::Object(o) => (
                o.get("repo")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| repo.clone()),
                o.get("number").and_then(Value::as_i64),
            ),
            _ => (None, None),
        };
        if let (Some(parent_repo), Some(number)) = (parent_repo, number.filter(|n| *n > 0)) {
            if valid_repo(&parent_repo) {
                links.push(LinkRef::issue_or_pr("issue", parent_repo, number, "parent"));
            }
        }
    }
    for parent_task in [payload.get("parent_task_id"), payload.get("parent_task")]
        .into_iter()
        .flatten()
    {
        let parent_id = parent_task
            .as_i64()
            .or_else(|| parent_task.get("task_id").and_then(Value::as_i64))
            .or_else(|| parent_task.get("id").and_then(Value::as_i64));
        if let Some(parent_id) = parent_id.filter(|n| *n > 0) {
            links.push(LinkRef::task(parent_id, "parent"));
        }
    }
    links.sort_by(|a, b| {
        (&a.link_kind, &a.other_kind, &a.link_key).cmp(&(&b.link_kind, &b.other_kind, &b.link_key))
    });
    links.dedup_by(|a, b| {
        a.link_kind == b.link_kind && a.other_kind == b.other_kind && a.link_key == b.link_key
    });
    links
}

fn link_insert_args(task_id: i64, link: &LinkRef, actor: &str, created_at: &str) -> Vec<Arg> {
    vec![
        Arg::Int(task_id),
        Arg::Text(link.other_kind.to_owned()),
        link.repo.clone().map(Arg::Text).unwrap_or(Arg::Null),
        link.number.map(Arg::Int).unwrap_or(Arg::Null),
        link.other_task_id.map(Arg::Int).unwrap_or(Arg::Null),
        Arg::Text(link.link_kind.clone()),
        Arg::Text(link.link_key.clone()),
        Arg::Text(actor.to_owned()),
        Arg::Text(created_at.to_owned()),
    ]
}

fn backfill_link_statement(
    task_id: i64,
    link: &LinkRef,
    created_at: &str,
) -> (&'static str, Vec<Arg>) {
    (
        "INSERT INTO queue_links \
         (task_id, other_kind, other_repo, other_number, other_task_id, link_kind, link_key, created_by, created_at) \
         SELECT ?, ?, ?, ?, ?, ?, ?, 'backfill', ? \
         WHERE NOT EXISTS (SELECT 1 FROM queue_links WHERE task_id = ? AND link_kind = ? AND link_key = ?)",
        vec![
            Arg::Int(task_id),
            Arg::Text(link.other_kind.to_owned()),
            link.repo.clone().map(Arg::Text).unwrap_or(Arg::Null),
            link.number.map(Arg::Int).unwrap_or(Arg::Null),
            link.other_task_id.map(Arg::Int).unwrap_or(Arg::Null),
            Arg::Text(link.link_kind.clone()),
            Arg::Text(link.link_key.clone()),
            Arg::Text(created_at.to_owned()),
            Arg::Int(task_id),
            Arg::Text(link.link_kind.clone()),
            Arg::Text(link.link_key.clone()),
        ],
    )
}

pub(crate) fn seed_task_links(
    t: &TursoBackend,
    task_id: i64,
    row: &Value,
    actor: &str,
) -> Result<(), TErr> {
    let created_at = row
        .get("created_at")
        .and_then(Value::as_str)
        .unwrap_or_else(|| "");
    let links = task_link_refs(row);
    if links.is_empty() {
        return Ok(());
    }
    let mut statements = vec![("BEGIN IMMEDIATE", Vec::new())];
    for link in &links {
        statements.push((
            "INSERT OR IGNORE INTO queue_links \
             (task_id, other_kind, other_repo, other_number, other_task_id, link_kind, link_key, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            link_insert_args(task_id, link, actor, created_at),
        ));
    }
    statements.push(("COMMIT", Vec::new()));
    t.exec_batch(&statements)?;
    Ok(())
}

fn normalized_dep(
    raw: &Value,
    task_id: i64,
    block_id: &str,
    fallback_created_at: &str,
    source: &str,
) -> Value {
    let kind = raw.get("kind").and_then(Value::as_str).unwrap_or("");
    let repo = raw
        .get("repo")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| valid_repo(r));
    let ref_number = match kind {
        "issue" | "pr" => raw.get("number").and_then(Value::as_i64).filter(|n| *n > 0),
        "ci" => raw.get("run_id").and_then(Value::as_i64).filter(|n| *n > 0),
        _ => None,
    };
    let ref_task_id = if kind == "task" {
        raw.get("task_id")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0)
    } else {
        None
    };
    let valid = match kind {
        "issue" | "pr" | "ci" => repo.is_some() && ref_number.is_some(),
        "task" => ref_task_id.is_some(),
        _ => false,
    };
    let raw_state = raw.get("state").and_then(Value::as_str).unwrap_or("open");
    let state = if !valid || !matches!(raw_state, "open" | "cleared" | "broken") {
        "malformed"
    } else if raw_state == "broken" {
        "cleared"
    } else {
        raw_state
    };
    let dep_kind = match kind {
        "issue" | "pr" | "task" | "ci" => format!("blocked-by-{kind}"),
        _ => "malformed".to_owned(),
    };
    let dep_key = if valid {
        dep_key(raw)
    } else {
        format!(
            "malformed:{}",
            serde_json::to_string(raw).unwrap_or_default()
        )
    };
    let created_at = raw
        .get("recorded_at")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_created_at)
        .to_owned();
    let cleared_at = raw
        .get("cleared_at")
        .and_then(Value::as_str)
        .or_else(|| {
            if raw_state == "broken" {
                raw.get("last_checked_at").and_then(Value::as_str)
            } else {
                None
            }
        })
        .map(str::to_owned);
    let mut detail = raw.clone();
    if !detail.is_object() {
        detail = json!({"raw": raw});
    }
    let detail_obj = detail.as_object_mut().expect("dependency detail object");
    detail_obj.insert("source".into(), json!(source));
    if raw_state == "broken" {
        detail_obj.insert("original_state".into(), json!("broken"));
    }
    if !valid || state == "malformed" {
        detail_obj.insert(
            "malformed_reason".into(),
            json!("dependency reference is incomplete, unknown, or has an unknown state"),
        );
        detail_obj.insert("raw_entry".into(), raw.clone());
    }
    json!({
        "task_id": task_id,
        "block_id": block_id,
        "dep_kind": dep_kind,
        "dep_key": dep_key,
        "ref_repo": repo,
        "ref_number": ref_number,
        "ref_task_id": ref_task_id,
        "state": state,
        "created_at": created_at,
        "cleared_at": cleared_at,
        "cleared_by": raw.get("cleared_by").cloned().unwrap_or(Value::Null),
        "detail": detail,
    })
}

fn dep_insert_statement(dep: &Value) -> (&'static str, Vec<Arg>) {
    (
        "INSERT INTO task_deps \
         (task_id, block_id, dep_kind, dep_key, ref_repo, ref_number, ref_task_id, state, created_at, cleared_at, cleared_by, detail) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(task_id, block_id, dep_key) DO UPDATE SET \
           dep_kind=excluded.dep_kind, ref_repo=excluded.ref_repo, ref_number=excluded.ref_number, \
           ref_task_id=excluded.ref_task_id, state=excluded.state, cleared_at=excluded.cleared_at, \
           cleared_by=excluded.cleared_by, detail=excluded.detail",
        vec![
            Arg::Int(dep["task_id"].as_i64().unwrap_or(0)),
            Arg::Text(dep["block_id"].as_str().unwrap_or_default().to_owned()),
            Arg::Text(dep["dep_kind"].as_str().unwrap_or("malformed").to_owned()),
            Arg::Text(dep["dep_key"].as_str().unwrap_or_default().to_owned()),
            dep["ref_repo"]
                .as_str()
                .map(|s| Arg::Text(s.to_owned()))
                .unwrap_or(Arg::Null),
            dep["ref_number"]
                .as_i64()
                .map(Arg::Int)
                .unwrap_or(Arg::Null),
            dep["ref_task_id"]
                .as_i64()
                .map(Arg::Int)
                .unwrap_or(Arg::Null),
            Arg::Text(dep["state"].as_str().unwrap_or("malformed").to_owned()),
            Arg::Text(dep["created_at"].as_str().unwrap_or_default().to_owned()),
            dep["cleared_at"]
                .as_str()
                .map(|s| Arg::Text(s.to_owned()))
                .unwrap_or(Arg::Null),
            dep["cleared_by"]
                .as_str()
                .map(|s| Arg::Text(s.to_owned()))
                .unwrap_or(Arg::Null),
            Arg::Text(dep["detail"].to_string()),
        ],
    )
}

const BACKFILL_DEP_INSERT_SQL: &str = "INSERT INTO task_deps \
    (task_id, block_id, dep_kind, dep_key, ref_repo, ref_number, ref_task_id, state, created_at, cleared_at, cleared_by, detail) \
    SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ? \
    WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ? AND status = 'blocked' \
      AND blocked_deps = ? AND dep_blocked_until IS ?) \
    ON CONFLICT(task_id, block_id, dep_key) DO UPDATE SET \
      dep_kind=excluded.dep_kind, ref_repo=excluded.ref_repo, ref_number=excluded.ref_number, \
      ref_task_id=excluded.ref_task_id, state=excluded.state, cleared_at=excluded.cleared_at, \
      cleared_by=excluded.cleared_by, detail=excluded.detail";

/// Prepare one blocked-deps cache generation for an enclosing write batch.
pub(crate) fn prepare_task_deps(
    task_id: i64,
    block_id: &str,
    deps: &[Value],
    fallback_created_at: &str,
    source: &str,
) -> (Vec<Value>, Vec<(&'static str, Vec<Arg>)>) {
    let normalized: Vec<Value> = deps
        .iter()
        .map(|dep| normalized_dep(dep, task_id, block_id, fallback_created_at, source))
        .collect();
    let statements = normalized.iter().map(dep_insert_statement).collect();
    (
        normalized.iter().map(dependency_cache_state).collect(),
        statements,
    )
}

/// Standalone normalized write for migration fixtures and simple callers.
pub(crate) fn sync_task_deps(
    t: &TursoBackend,
    task_id: i64,
    block_id: &str,
    deps: &[Value],
    fallback_created_at: &str,
    source: &str,
) -> Result<Vec<Value>, TErr> {
    let (cache, writes) = prepare_task_deps(task_id, block_id, deps, fallback_created_at, source);
    if !writes.is_empty() {
        let mut statements = vec![("BEGIN IMMEDIATE", Vec::new())];
        statements.extend(writes);
        statements.push(("COMMIT", Vec::new()));
        t.exec_batch(&statements)?;
    }
    Ok(cache)
}

fn dependency_cache_state(dep: &Value) -> Value {
    let state = dep
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("malformed");
    let mut raw = dep.get("detail").cloned().unwrap_or_else(|| json!({}));
    if !raw.is_object() {
        raw = json!({"raw": raw});
    }
    let obj = raw.as_object_mut().expect("dependency detail object");
    let dep_kind = dep
        .get("dep_kind")
        .and_then(Value::as_str)
        .unwrap_or("malformed");
    let kind = dep_kind.strip_prefix("blocked-by-").unwrap_or("malformed");
    obj.insert("kind".into(), json!(kind));
    if let Some(repo) = dep.get("ref_repo").and_then(Value::as_str) {
        obj.insert("repo".into(), json!(repo));
    }
    if let Some(number) = dep.get("ref_number").and_then(Value::as_i64) {
        let key = if kind == "ci" { "run_id" } else { "number" };
        obj.insert(key.into(), json!(number));
    }
    if let Some(task_id) = dep.get("ref_task_id").and_then(Value::as_i64) {
        obj.insert("task_id".into(), json!(task_id));
    }
    let cache_state = match state {
        "malformed" => "broken",
        "cleared" if obj.get("original_state").and_then(Value::as_str) == Some("broken") => {
            "broken"
        }
        other => other,
    };
    obj.insert("state".into(), json!(cache_state));
    obj.insert(
        "recorded_at".into(),
        dep.get("created_at").cloned().unwrap_or(Value::Null),
    );
    obj.insert(
        "cleared_at".into(),
        dep.get("cleared_at").cloned().unwrap_or(Value::Null),
    );
    raw
}

pub(crate) fn clear_task_deps_statement(
    task_id: i64,
    block_id: Option<&str>,
    worker: &str,
    reason: &str,
    now: &str,
) -> (&'static str, Vec<Arg>) {
    let block_filter = if block_id.is_some() {
        " AND block_id = ?"
    } else {
        ""
    };
    let mut args = vec![
        Arg::Text(now.to_owned()),
        Arg::Text(worker.to_owned()),
        Arg::Text(reason.to_owned()),
        Arg::Int(task_id),
    ];
    if let Some(block_id) = block_id {
        args.push(Arg::Text(block_id.to_owned()));
    }
    let sql = if block_filter.is_empty() {
        "UPDATE task_deps SET state = 'cleared', cleared_at = ?, cleared_by = ?, \
         detail = json_set(detail, '$.clear_reason', ?) WHERE task_id = ? AND state = 'open'"
    } else {
        "UPDATE task_deps SET state = 'cleared', cleared_at = ?, cleared_by = ?, \
         detail = json_set(detail, '$.clear_reason', ?) WHERE task_id = ? \
         AND state = 'open' AND block_id = ?"
    };
    (sql, args)
}

pub(crate) fn clear_missing_task_deps_statement(
    task_id: i64,
    block_id: &str,
    deps: &[Value],
    worker: &str,
    reason: &str,
    now: &str,
) -> (String, Vec<Arg>) {
    let keys: std::collections::HashSet<String> = deps
        .iter()
        .map(|dep| {
            dep.get("dep_key")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| dep_key(dep))
        })
        .collect();
    let placeholders = std::iter::repeat_n("?", keys.len())
        .collect::<Vec<_>>()
        .join(", ");
    let keep = if keys.is_empty() {
        String::new()
    } else {
        format!(" AND dep_key NOT IN ({placeholders})")
    };
    let mut args = vec![
        Arg::Text(now.to_owned()),
        Arg::Text(worker.to_owned()),
        Arg::Text(reason.to_owned()),
        Arg::Int(task_id),
        Arg::Text(block_id.to_owned()),
    ];
    args.extend(keys.into_iter().map(Arg::Text));
    (
        format!(
            "UPDATE task_deps SET state = 'cleared', cleared_at = ?, cleared_by = ?, \
             detail = json_set(detail, '$.clear_reason', ?) WHERE task_id = ? \
             AND block_id = ? AND state = 'open'{keep}"
        ),
        args,
    )
}

/// One-time idempotent Q6 backfill. New writes maintain the normalized
/// tables directly; the marker prevents startup from repeatedly scanning
/// the historical task set.
pub(crate) fn ensure_q6_backfill(t: &TursoBackend) -> Result<(), TErr> {
    let marker = t.query_objs(
        "SELECT name FROM queue_schema_migrations WHERE name = 'q6-links-deps-v1'",
        &[],
        &[],
    )?;
    if !marker.is_empty() {
        return Ok(());
    }

    let mut after_id = 0i64;
    let mut scanned = 0i64;
    let mut link_candidates = 0i64;
    let mut dep_candidates = 0i64;
    loop {
        let rows = t.query_objs(
            "SELECT id, payload, result, dedupe_key, status, blocked_deps, created_at, updated_at, \
             dep_blocked_until FROM tasks WHERE id > ? ORDER BY id LIMIT 200",
            &[Arg::Int(after_id)],
            &[],
        )?;
        if rows.is_empty() {
            break;
        }
        let mut statements = vec![("BEGIN IMMEDIATE", Vec::new())];
        for row in &rows {
            let task_id = row.get("id").and_then(Value::as_i64).unwrap_or(0);
            if task_id <= 0 {
                continue;
            }
            let created_at = row
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or("1970-01-01T00:00:00Z");
            for link in task_link_refs(row) {
                statements.push(backfill_link_statement(task_id, &link, created_at));
                link_candidates += 1;
            }
            let raw_blocked = row.get("blocked_deps").cloned().unwrap_or(Value::Null);
            if row.get("status").and_then(Value::as_str) != Some("blocked") {
                continue;
            }
            let parsed_blocked = parse_json(Some(&raw_blocked));
            let deps = match parsed_blocked {
                Value::Array(deps) => deps,
                Value::Null => match &raw_blocked {
                    Value::String(raw) if !raw.trim().is_empty() => {
                        vec![json!({"raw_blocked_deps": raw})]
                    }
                    _ => Vec::new(),
                },
                other => vec![other],
            };
            if deps.is_empty() {
                continue;
            }
            let block_id = row
                .get("dep_blocked_until")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    deps.first()
                        .and_then(|d| d.get("recorded_at"))
                        .and_then(Value::as_str)
                })
                .or_else(|| row.get("updated_at").and_then(Value::as_str))
                .unwrap_or(created_at);
            for raw in &deps {
                let dep = normalized_dep(raw, task_id, block_id, created_at, "backfill");
                let (_, mut args) = dep_insert_statement(&dep);
                args.extend([
                    Arg::Int(task_id),
                    Arg::Text(raw_blocked.as_str().unwrap_or_default().to_owned()),
                    row.get("dep_blocked_until")
                        .and_then(Value::as_str)
                        .map(|s| Arg::Text(s.to_owned()))
                        .unwrap_or(Arg::Null),
                ]);
                statements.push((BACKFILL_DEP_INSERT_SQL, args));
                dep_candidates += 1;
            }
        }
        statements.push(("COMMIT", Vec::new()));
        t.exec_batch(&statements)?;
        scanned += rows.len() as i64;
        after_id = rows
            .last()
            .and_then(|row| row.get("id").and_then(Value::as_i64))
            .unwrap_or(after_id);
    }
    t.exec(
        "INSERT OR IGNORE INTO queue_schema_migrations (name, applied_at, detail) VALUES (?, ?, ?)",
        &[
            Arg::Text("q6-links-deps-v1".to_owned()),
            Arg::Text(now_iso()),
            Arg::Text(
                json!({
                    "tasks_scanned": scanned,
                    "link_candidates": link_candidates,
                    "dep_candidates": dep_candidates,
                })
                .to_string(),
            ),
        ],
    )?;
    Ok(())
}

fn link_row(
    t: &TursoBackend,
    task_id: i64,
    link: &LinkRef,
    include_removed: bool,
) -> Result<Option<Value>, TErr> {
    let removed = if include_removed {
        ""
    } else {
        " AND removed_at IS NULL"
    };
    let sql = format!(
        "SELECT id, task_id, other_kind, other_repo, other_number, other_task_id, \
         link_kind, link_key, created_by, created_at, removed_at, removed_by \
         FROM queue_links WHERE task_id = ? AND link_kind = ? AND link_key = ?{removed} \
         ORDER BY id DESC LIMIT 1"
    );
    Ok(t.query_objs(
        &sql,
        &[
            Arg::Int(task_id),
            Arg::Text(link.link_kind.clone()),
            Arg::Text(link.link_key.clone()),
        ],
        &[],
    )?
    .into_iter()
    .next())
}

fn link_event_detail(link: &LinkRef, task: &Value, actor: &str) -> Value {
    super::ctx_scope::stamp(&with_marks(
        &json!({
            "link_kind": link.link_kind,
            "other_kind": link.other_kind,
            "other_repo": link.repo,
            "other_number": link.number,
            "other_task_id": link.other_task_id,
            "link_key": link.link_key,
            "actor": actor,
        }),
        task,
    ))
}

/// Add one queue relationship. A duplicate active add is an idempotent no-op.
pub(crate) fn verb_link_add(t: &TursoBackend, req: &Value) -> Value {
    verb_link_change(t, req, true)
}

/// Soft-remove one active queue relationship.
pub(crate) fn verb_link_remove(t: &TursoBackend, req: &Value) -> Value {
    verb_link_change(t, req, false)
}

fn verb_link_change(t: &TursoBackend, req: &Value, add: bool) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    if task_id <= 0 {
        return fail(2, "link: task_id must be positive");
    }
    let link = match request_link(req) {
        Ok(link) => link,
        Err(error) => return fail(2, error),
    };
    let worker = req_str(req, "worker")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("fleet queue link")
        .to_owned();
    let task = match get_task(t, task_id) {
        Ok(Some(task)) => task,
        Ok(None) => return fail(5, format!("link: task {task_id} does not exist")),
        Err(error) => return terr(error),
    };
    let existing = match link_row(t, task_id, &link, false) {
        Ok(row) => row,
        Err(error) => return terr(error),
    };
    if add {
        if let Some(row) = existing.as_ref() {
            return ok(json!({"row": row.clone(), "changed": false}));
        }
    } else if existing.is_none() {
        return fail(
            5,
            format!(
                "link: task {task_id} has no active {} link {}",
                link.link_kind, link.link_key
            ),
        );
    }

    let now = now_iso();
    let detail = link_event_detail(&link, &task, &worker);
    let mut statements = vec![("BEGIN IMMEDIATE", Vec::new())];
    if add {
        statements.push((
            "INSERT OR IGNORE INTO queue_links \
             (task_id, other_kind, other_repo, other_number, other_task_id, link_kind, link_key, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            link_insert_args(task_id, &link, &worker, &now),
        ));
    } else {
        let link_id = existing
            .as_ref()
            .and_then(|row| row.get("id").and_then(Value::as_i64))
            .unwrap_or(0);
        statements.push((
            "UPDATE queue_links SET removed_at = ?, removed_by = ? \
             WHERE id = ? AND removed_at IS NULL",
            vec![
                Arg::Text(now.clone()),
                Arg::Text(worker.clone()),
                Arg::Int(link_id),
            ],
        ));
    }
    let event_kind = if add { "linked" } else { "unlinked" };
    let event_sql = if add {
        "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
         SELECT ?, ?, 'linked', ?, ? WHERE changes() = 1"
    } else {
        "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
         SELECT ?, ?, 'unlinked', ?, ? WHERE changes() = 1"
    };
    statements.push((
        event_sql,
        vec![
            Arg::Text(worker),
            Arg::Int(task_id),
            Arg::Text(detail.to_string()),
            Arg::Text(now),
        ],
    ));
    statements.push((HISTORY_INSERT_SQL, vec![Arg::Null, Arg::Null]));
    statements.push(("COMMIT", Vec::new()));
    let results = match t.exec_batch(&statements) {
        Ok(results) => results,
        Err(error) => return terr(error),
    };
    let changed = results.get(1).is_some_and(|r| r.affected > 0);
    let row = match link_row(t, task_id, &link, !add) {
        Ok(row) => row,
        Err(error) => return terr(error),
    };
    let Some(row) = row else {
        return fail(5, format!("link: {event_kind} was not recorded"));
    };
    ok(json!({"row": row, "changed": changed}))
}

pub(crate) fn verb_link_list(t: &TursoBackend, req: &Value) -> Value {
    let Some(task_id) = req_i64(req, "task_id").filter(|id| *id > 0) else {
        return fail(2, "link list: task_id must be positive");
    };
    match get_task(t, task_id) {
        Ok(Some(_)) => {}
        Ok(None) => return fail(5, format!("link list: task {task_id} does not exist")),
        Err(error) => return terr(error),
    }
    let include_removed = req
        .get("include_removed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let removed = if include_removed {
        ""
    } else {
        " AND removed_at IS NULL"
    };
    let sql = format!(
        "SELECT id, task_id, other_kind, other_repo, other_number, other_task_id, \
         link_kind, link_key, created_by, created_at, removed_at, removed_by \
         FROM queue_links WHERE task_id = ?{removed} ORDER BY id"
    );
    match t.query_objs(&sql, &[Arg::Int(task_id)], &[]) {
        Ok(rows) => ok(json!({"task_id": task_id, "links": rows})),
        Err(error) => terr(error),
    }
}
