//! read-side repo-queue verbs: validate, enqueue, list, timeline, models (split from client.rs, #8588).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::*;

fn extract_repeatable(rest: &[String], flag: &str) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut values = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == flag {
            if i + 1 >= rest.len() || rest[i + 1].starts_with("--") {
                die(2, &format!("argument {flag}: expected one argument"));
            }
            values.push(rest[i + 1].clone());
            i += 2;
        } else if let Some(value) = rest[i].strip_prefix(&format!("{flag}=")) {
            values.push(value.to_string());
            i += 1;
        } else {
            kept.push(rest[i].clone());
            i += 1;
        }
    }
    (kept, values)
}

fn parse_meta_pairs(values: &[String]) -> Result<Value, String> {
    let mut object = serde_json::Map::new();
    for raw in values {
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got {raw:?}"))?;
        let parsed =
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));
        if object.insert(key.to_string(), parsed).is_some() {
            return Err(format!("duplicate key {key:?}"));
        }
    }
    let result = Value::Object(object);
    crate::validate::validate_metadata(&result)?;
    Ok(result)
}

pub(super) fn cmd_meta(rest: &[String]) -> i32 {
    let verb = rest.first().map(String::as_str).unwrap_or("");
    let rest = &rest[1..];
    let args = parse_verb_args("meta", rest, &["json"], &[]);
    if verb == "keys" {
        if args.pos(0).is_some() || args.get("json").is_some() {
            usage_error("meta", "meta keys takes no arguments");
        }
        reject_extra_pos("meta", &args, 1);
        let resp = unwrap_ok(call("/meta", &json!({"op":"keys"})));
        print_json(resp.get("row").unwrap_or(&Value::Null));
        return 0;
    }
    match verb {
        "get" => {
            reject_extra_pos("meta", &args, 2);
            let id = args.pos_i64(0, "task_id");
            let mut req = json!({"op":"get","task_id":id});
            if let Some(key) = args.pos(1) {
                req["key"] = json!(key);
            }
            let resp = unwrap_ok(call("/meta", &req));
            print_json(resp.get("row").unwrap_or(&Value::Null));
        }
        "set" => {
            if let Some(raw) = args.get("json") {
                reject_extra_pos("meta", &args, 1);
                let patch: Value = serde_json::from_str(&raw)
                    .unwrap_or_else(|e| die(2, &format!("meta set --json: invalid JSON: {e}")));
                if !patch.is_object() {
                    die(2, "meta set --json: expected a JSON object");
                }
                if let Err(e) = crate::validate::validate_metadata_patch(&patch) {
                    die(2, &e);
                }
                let id = args.pos_i64(0, "task_id");
                let resp = unwrap_ok(call(
                    "/meta",
                    &json!({"op":"set","task_id":id,"patch":patch}),
                ));
                print_json(resp.get("row").unwrap_or(&Value::Null));
            } else {
                reject_extra_pos("meta", &args, 3);
                let id = args.pos_i64(0, "task_id");
                let key = args
                    .pos(1)
                    .unwrap_or_else(|| die(2, "meta set: key required"));
                let raw = args
                    .pos(2)
                    .unwrap_or_else(|| die(2, "meta set: JSON value required"));
                let value: Value = serde_json::from_str(&raw)
                    .unwrap_or_else(|e| die(2, &format!("meta set: invalid JSON value: {e}")));
                let mut patch = serde_json::Map::new();
                patch.insert(key, value);
                let patch = Value::Object(patch);
                if let Err(e) = crate::validate::validate_metadata_patch(&patch) {
                    die(2, &e);
                }
                let resp = unwrap_ok(call(
                    "/meta",
                    &json!({"op":"set","task_id":id,"patch":patch}),
                ));
                print_json(resp.get("row").unwrap_or(&Value::Null));
            }
        }
        "unset" => {
            reject_extra_pos("meta", &args, 2);
            let id = args.pos_i64(0, "task_id");
            let key = args
                .pos(1)
                .unwrap_or_else(|| die(2, "meta unset: key required"));
            let resp = unwrap_ok(call("/meta", &json!({"op":"unset","task_id":id,"key":key})));
            print_json(resp.get("row").unwrap_or(&Value::Null));
        }
        _ => die(
            2,
            &format!("meta: expected get, set, unset, or keys\n{}", usage("meta")),
        ),
    }
    0
}

/// Stewardship staleness read (#14170 Q8): one broker call returns the
/// actionable tasks whose last history transition is older than the
/// threshold plus the claims near their lease cap.
pub(super) fn cmd_staleness(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "staleness",
        rest,
        &["queue", "older-than-seconds", "lease-cap-seconds", "limit"],
        &[],
    );
    reject_extra_pos("staleness", &args, 0);
    let mut req = json!({});
    if let Some(q) = args.get("queue") {
        req["queue"] = json!(q);
    }
    if let Some(v) = args.get_i64("older-than-seconds") {
        req["older_than_seconds"] = json!(v);
    }
    if let Some(v) = args.get_i64("lease-cap-seconds") {
        req["lease_cap_seconds"] = json!(v);
    }
    if let Some(v) = args.get_i64("limit") {
        req["limit"] = json!(v);
    }
    let resp = unwrap_ok(call("/staleness", &req));
    print_json(&resp);
    0
}

/// Stewardship needs-user read (#14170 Q8): blocked/parked tasks with an
/// open `question` metadata entry, oldest first.
pub(super) fn cmd_needs_user(rest: &[String]) -> i32 {
    let args = parse_verb_args("needs-user", rest, &["queue", "state", "limit"], &[]);
    reject_extra_pos("needs-user", &args, 0);
    let mut req = json!({});
    if let Some(q) = args.get("queue") {
        req["queue"] = json!(q);
    }
    if let Some(s) = args.get("state") {
        req["state"] = json!(s);
    }
    if let Some(v) = args.get_i64("limit") {
        req["limit"] = json!(v);
    }
    let resp = unwrap_ok(call("/needs-user", &req));
    print_json(&resp);
    0
}

pub(super) fn cmd_validate(rest: &[String]) -> i32 {
    let args = parse_verb_args("validate", rest, &[], &[]);
    reject_extra_pos("validate", &args, 1);
    let path = args
        .pos
        .first()
        .cloned()
        .unwrap_or_else(|| die(2, "validate: packet required"));
    let packet = load_packet(&path);
    validate_via_broker(&packet, false);
    print_json(&json!({
        "valid": true,
        "dedupe_key": packet.get("dedupe_key").cloned().unwrap_or(Value::Null),
        "kind": packet.get("payload").and_then(|p| p.get("kind")).cloned().unwrap_or(Value::Null),
        "title": packet.get("title").cloned().unwrap_or(Value::Null),
    }));
    0
}

/// Enqueue queue resolution (#7016, the shadow-default trap): the
/// --queue flag wins, then the packet's own declared queue, then
/// the REPO_QUEUE_DEFAULT_QUEUE env. There is deliberately NO
/// built-in default for enqueue: with none of the three the enqueue
/// fails loudly instead of silently landing in the shadow queue —
/// the failure that misrouted ~291 tasks on 2026-11-07. Pure, so
/// the resolution order is unit-testable.
pub(super) fn resolve_enqueue_queue(
    flag: Option<&str>,
    packet: &Value,
    env_default: Option<&str>,
) -> Result<String, String> {
    let packet_queue = packet
        .get("queue")
        .and_then(|q| q.as_str())
        .map(str::trim)
        .filter(|q| !q.is_empty());
    for candidate in [flag, packet_queue, env_default] {
        if let Some(q) = candidate.map(str::trim).filter(|q| !q.is_empty()) {
            return Ok(q.to_string());
        }
    }
    Err(
        "enqueue: no queue resolved — pass --queue, declare \"queue\" in the \
         packet, or set REPO_QUEUE_DEFAULT_QUEUE (enqueue has no default queue; \
         the old shadow default silently misrouted work)"
            .to_string(),
    )
}

pub(super) fn cmd_enqueue(rest: &[String]) -> i32 {
    let (rest, meta_args) = extract_repeatable(rest, "--meta");
    let metadata = match parse_meta_pairs(&meta_args) {
        Ok(value) => value,
        Err(e) => die(2, &format!("enqueue --meta: {e}")),
    };
    let args = parse_verb_args(
        "enqueue",
        &rest,
        &["queue", "platform", "meta"],
        &["dry-run", "shadow-intent"],
    );
    reject_extra_pos("enqueue", &args, 1);
    let path = args
        .pos
        .first()
        .cloned()
        .unwrap_or_else(|| die(2, "enqueue: packet required"));
    let packet = load_packet(&path);
    let dry_run = args.has("dry-run");
    validate_via_broker(&packet, dry_run);
    let env_default = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    let queue = match resolve_enqueue_queue(
        args.get("queue").as_deref(),
        &packet,
        env_default.as_deref(),
    ) {
        Ok(q) => q,
        Err(e) => die(2, &e),
    };
    // #8097 Rule 2: the platform selector is optional; when given
    // it restricts the item to that family. Absent = unset.
    let platform = match args.get("platform") {
        None => None,
        Some(p) => match platform_flag(&p) {
            Ok(v) => Some(v),
            Err(e) => die(2, &e),
        },
    };
    let shadow_intent = args.has("shadow-intent");
    if queue.ends_with("-shadow") && !shadow_intent {
        die(
            2,
            &format!(
                "enqueue: queue {queue} is a shadow queue — pass --shadow-intent \
                 to enqueue there deliberately"
            ),
        );
    }
    let call_started = now_micros();
    let resp = call(
        "/enqueue",
        &json!({
            "packet": packet, "queue": queue, "dry_run": dry_run,
            "shadow_intent": shadow_intent, "platform": platform, "metadata": metadata,
        }),
    );
    let resp = unwrap_ok(resp);
    if dry_run {
        let mut task = resp.get("task").cloned().unwrap_or(json!({}));
        if let Some(o) = task.as_object_mut() {
            o.insert("_dry_run".to_string(), Value::Bool(true));
            o.insert("_rpc".to_string(), json!("agent.enqueue_repo_task"));
        }
        print_json(&task);
        return 0;
    }
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    // Outcome classification (mirrors _enqueue_outcome in repo-queue.py).
    let tid = row.get("id").cloned().unwrap_or(Value::Null);
    let q = row
        .get("queue")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let s = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_raw = row.get("created_at").cloned().unwrap_or(Value::Null);
    let created = parse_ts(&created_raw);
    let updated = parse_ts(&row.get("updated_at").cloned().unwrap_or(Value::Null));
    let skew = 10_000_000i64; // 10 s in micros
    let mut warnings: Vec<String> = Vec::new();
    let line;
    if let Some(c) = resp.get("work_identity_collision") {
        // #10627: the broker found a live task already holding this
        // work identity (repo, issue, kind family) and filed nothing.
        line = format!(
            "work already queued: existing task {} (queue {q}, status {s}) holds {}; nothing filed \
             (set payload.dedupe_override to file a deliberate parallel)",
            fmt_cell(Some(&tid)),
            fmt_cell(c.get("identity")),
        );
        if q != queue {
            warnings.push(format!(
                "the holding task is in a different queue ({q} != requested {queue})"
            ));
        }
    } else if let Some(pred) = resp.get("revived_from") {
        // Broker-side revive (#6838): the key's prior task was
        // terminal, so this enqueue created a fresh task linked to
        // its predecessor.
        line = format!(
            "revived: new task {} (queue {q}, status {s}), predecessor task {} ({})",
            fmt_cell(Some(&tid)),
            fmt_cell(pred.get("task_id")),
            fmt_cell(pred.get("status")),
        );
    } else if created.map(|c| c >= call_started - skew).unwrap_or(false) {
        line = format!(
            "enqueued new task {} (queue {q}, status {s})",
            fmt_cell(Some(&tid))
        );
    } else {
        if q != queue {
            warnings.push(format!(
                "dedupe hit is in a different queue ({q} != requested {queue})"
            ));
        }
        if s == "queued" && updated.map(|u| u >= call_started - skew).unwrap_or(false) {
            line = format!(
                "revived: existing task {} (queue {q}, status {s}, created {})",
                fmt_cell(Some(&tid)),
                fmt_cell(Some(&created_raw))
            );
        } else {
            if TERMINAL_STATUSES.contains(&s.as_str()) {
                warnings.push(format!("dedupe hit is in a terminal state (status {s})"));
            }
            line = format!(
                "dedupe hit: existing task {} (queue {q}, status {s}, created {})",
                fmt_cell(Some(&tid)),
                fmt_cell(Some(&created_raw))
            );
        }
    }
    println!("{line}");
    for w in &warnings {
        println!("warning: {w}");
    }
    print_json(&row);
    0
}

/// One `/list` row as the printed table cells, in column order
/// (task, priority, status, age, dedupe_key, title). Pure so the
/// claimed-list recovery contract (#5486) is unit-testable: the cells
/// are built from exactly the identifying fields the broker returns.
pub(super) fn list_row_cells(r: &Value) -> Vec<String> {
    vec![
        format!("task {}", fmt_cell(r.get("id"))),
        fmt_cell(r.get("priority")),
        format!(
            "{}{}",
            fmt_cell(r.get("status")),
            if r.get("manual_only").and_then(Value::as_bool) == Some(true) {
                " (manual-only)"
            } else {
                ""
            }
        ),
        age_str(&r.get("created_at").cloned().unwrap_or(Value::Null)),
        fmt_cell(r.get("dedupe_key")),
        truncate(&fmt_cell(r.get("title")), 72),
    ]
}

/// Find a task row by id in a `/list` rows array — the recovery path a
/// worker uses when a claim's own output was lost: `list --status
/// claimed` must return the same task with its identifying fields.
pub(super) fn find_task_row(rows: &[Value], task_id: i64) -> Option<&Value> {
    rows.iter()
        .find(|r| r.get("id").and_then(|v| v.as_i64()) == Some(task_id))
}

/// The scope note for a bare `list` (#10838): with no filter flag
/// the broker answers in its default ordering for one queue
/// (newest terminal rows first), which reads as "nothing to do".
/// Pure over the parsed args so the trigger is unit-testable.
pub(crate) fn bare_list_note(args: &Args, resolved_queue: Option<&str>) -> Option<String> {
    for key in ["queue", "status", "not-before", "lane", "worker", "task"] {
        if args.opts.contains_key(key) {
            return None;
        }
    }
    let scope = match resolved_queue {
        Some(queue) => format!("queue {queue:?}"),
        None => "every queue".to_string(),
    };
    Some(format!(
        "note: bare `list` shows {scope} in the broker's default ordering (newest terminal rows first) — not the actionable set. Use --status queued|parked|claimed|blocked for actionable work, --queue <name> to pick a stream (e.g. repo-health-code-pilot), --limit <n> to widen the window."
    ))
}

pub(super) fn cmd_list(rest: &[String]) -> i32 {
    let (rest, meta_args) = extract_repeatable(rest, "--meta");
    let metadata_filter = match parse_meta_pairs(&meta_args) {
        Ok(value) => value,
        Err(e) => die(2, &format!("list --meta: {e}")),
    };
    let args = parse_verb_args(
        "list",
        &rest,
        &[
            "queue",
            "status",
            "limit",
            "not-before",
            "lane",
            "worker",
            "task",
            "meta",
            "label",
        ],
        &["json"],
    );
    reject_extra_pos("list", &args, 0);
    let queue = opt_queue_scoped(&args);
    // #8581 exact-task status pilot: --json with --queue + --task
    // is a different read, not a rendering of this listing — one
    // scoped broker read, one JSON record, no paging.
    if args.has("json") {
        if !metadata_filter.as_object().is_some_and(|m| m.is_empty()) {
            usage_error("list", "--json exact status does not support --meta");
        }
        if args.get("label").is_some() {
            usage_error("list", "--json exact status does not support --label");
        }
        return cmd_list_exact_status(&args, queue);
    }
    if let Some(note) = bare_list_note(&args, queue.as_deref()) {
        eprintln!("repo-queue: {note}");
    }
    let limit = match list_limit_or_default(args.get_i64("limit")) {
        Ok(limit) => limit,
        Err(e) => die(2, &e),
    };
    // #9027: --not-before is validated with the same rule park and
    // revive apply (#6426). Forwarded raw, a garbage value silently
    // narrowed the listing on the Turso backend, whose TEXT
    // comparison made the predicate false for every row carrying a
    // not_before (or disabled the filter) with exit 0.
    let not_before = match args.get("not-before") {
        Some(nb) => match validate_not_before_flag(&nb) {
            Ok(v) => Some(v),
            Err(e) => die(2, &e),
        },
        None => None,
    };
    // #7871: fetch in pages of LIST_PAGE_SIZE (see list_pages) — a
    // single oversized /list response comes back unusable and used
    // to render as a silent "(no tasks)". `--task <id>` narrows to
    // one task regardless of status ordering, so a worker can
    // verify a just-completed task's status from list output even
    // when the done list is thousands of rows long.
    let mut rows: Vec<Value> = Vec::new();
    let mut total: Option<i64> = None;
    for (offset, page_limit) in list_pages(limit) {
        let resp = call(
            "/list",
            &json!({
                "queue": queue,
                "status": args.get("status"),
                "limit": page_limit,
                "offset": offset,
                "not_before": not_before,
                "lane": args.get("lane"),
                "worker": args.get("worker"),
                "task": args.get_i64("task"),
                "meta": metadata_filter.clone(),
                "label": args.get("label"),
            }),
        );
        let resp = unwrap_ok(resp);
        if let Some(t) = list_total(&resp) {
            total = Some(t);
        }
        let page = match list_rows(&resp) {
            Ok(page) => page,
            Err(e) => die(3, &e),
        };
        let short_page = (page.len() as i64) < page_limit;
        rows.extend(page);
        if short_page {
            break;
        }
    }
    if rows.is_empty() {
        println!("(no tasks)");
        return 0;
    }
    let cols: Vec<String> = ["task", "priority", "status", "age", "dedupe_key", "title"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let str_rows: Vec<Vec<String>> = rows.iter().map(list_row_cells).collect();
    print_table(&cols, &str_rows);
    // #10376: name the completeness of the listing — a limited
    // window must never read as the whole matching set.
    if let Some(t) = total {
        if (rows.len() as i64) < t {
            println!(
                "(truncated: showing {} of {} matching tasks — raise --limit to see more)",
                rows.len(),
                t
            );
        } else {
            println!("({} matching tasks)", t);
        }
    }
    0
}

/// Show one task's rich current broker row and the tail of its audit
/// timeline. The broker's rich /list row is the source of truth for
/// mutable state; events are intentionally limited only after the
/// same normalization and ordering used by `timeline`.
pub(super) fn cmd_show(rest: &[String]) -> i32 {
    let args = parse_verb_args("show", rest, &["limit"], &["json"]);
    reject_extra_pos("show", &args, 1);
    let task_id = args.pos_i64(0, "task_id");
    let limit = match list_limit_or_default(args.get_i64("limit").or(Some(10))) {
        Ok(limit) => limit as usize,
        Err(e) => die(2, &e),
    };
    let resp = unwrap_ok(call(
        "/list",
        &json!({"rich": true, "task": task_id, "limit": 1}),
    ));
    let rows = match list_rows(&resp) {
        Ok(rows) => rows,
        Err(e) => die(3, &e),
    };
    let Some(task) = rows
        .into_iter()
        .find(|r| r.get("id").and_then(Value::as_i64) == Some(task_id))
    else {
        eprint(&format!("repo-queue show: task {task_id} not found\n"));
        return 1;
    };
    let mut events = Vec::new();
    for kind in [
        "claimed",
        "staleness_autocomplete",
        "session_bound",
        "lane_session_bound",
        "model_stamp",
        "session_unbound",
        "session_binding_cleared",
        "completed",
        "lease_expired",
        "lease_expired_at_cap",
        "claim_refused",
        "released",
        "parked",
        "reprioritized",
        "reprioritize_sync_pending",
        "failed",
        "cancelled",
    ] {
        let response = call(
            "/events-list",
            &json!({"task_id": task_id, "kind": kind, "limit": 1000}),
        );
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            eprint("repo-queue show: broker does not serve /events-list yet\n");
            return 3;
        }
        if let Some(rows) = response
            .get("row")
            .and_then(|r| r.get("rows"))
            .and_then(Value::as_array)
        {
            events.extend(rows.iter().filter_map(timeline_event));
        }
    }
    events.sort_by_key(|event| event.id);
    let events = collapse_timeline(events);
    let start = events.len().saturating_sub(limit);
    let history = &events[start..];
    // #14170 Q4: the latest notes inline, bounded, with the count of
    // live notes so a reader knows how many the bound left out.
    let notes = fetch_notes(task_id, Some(SHOW_NOTES_LATEST));
    if args.has("json") {
        let (shown, count) = match &notes {
            Some((n, c)) => (n.iter().map(note_json).collect::<Vec<_>>(), json!(c)),
            None => (Vec::new(), Value::Null),
        };
        print_json(&json!({
            "task": task,
            "notes": shown,
            "note_count": count,
            "history": history.iter().map(|ev| json!({
                "id": ev.id, "kind": ev.kind, "created_at": ev.created_at,
                "session_id": ev.session_id, "provider": ev.provider,
                "model": ev.model, "variant": ev.variant, "trigger": ev.trigger,
            })).collect::<Vec<_>>(),
        }));
        return 0;
    }
    println!("task {task_id}");
    for key in [
        "title",
        "status",
        "kind",
        "queue",
        "pool",
        "attempts",
        "max_attempts",
        "worker_id",
        "lease_until",
        "park_kind",
        "wait_kind",
        "ready_condition",
        "not_before",
        "blocker",
        "last_error",
        "lane_id",
        "lane_reason",
        "platform",
        "claim_provider",
        "claim_model",
        "claim_variant",
        "claim_session_id",
        "issue",
        "pr",
        "labels",
        "labels_synced_at",
        "progress",
    ] {
        let payload = task.get("payload");
        let progress = task.get("progress");
        let value = task.get(key).or_else(|| match key {
            "kind" => payload.and_then(|p| p.get("kind")),
            "issue" | "pr" => payload.and_then(|p| p.get(key)),
            "blocker" | "lane_reason" | "wait_kind" | "ready_condition" => {
                progress.and_then(|p| p.get(key))
            }
            _ => None,
        });
        if let Some(value) = value {
            println!("{key}: {}", value);
        }
    }
    if let Some(checkpoint) = task.get("progress").and_then(|p| p.get("last_checkpoint")) {
        println!("latest_checkpoint: {checkpoint}");
    }
    if let Some(note) = task.get("progress").and_then(|p| {
        p.get("last_checkpoint")
            .and_then(|c| c.get("note"))
            .or_else(|| p.get("note"))
    }) {
        println!("latest_checkpoint_note: {note}");
    }
    println!("history ({} most recent events):", history.len());
    for event in history {
        println!("{}", format_timeline_event(event));
    }
    if let Some((shown, count)) = &notes {
        if *count > 0 || !shown.is_empty() {
            println!("notes ({} most recent of {count}):", shown.len());
            for n in shown {
                println!("{}", format_timeline_note(n));
            }
        }
    }
    0
}

/// How many of a task's newest notes `show` prints inline.
pub(super) const SHOW_NOTES_LATEST: i64 = 5;

/// The /claim-flow request invariants (#5757-A), checked before
/// any step runs, on both broker backends (the broker is the
/// trust boundary; the client mirrors the same rules in
/// claim_bind_path): a session to bind is required, batch claims
/// bind one session per item elsewhere, and --from-session
/// transfers keep the three-verb sequence. Returns the session
/// id to bind.
pub(crate) fn claim_flow_precheck(req: &Value) -> Result<String, String> {
    let session = req.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
    if session.is_empty() {
        return Err("claim-flow: session_id is required (the session the claimed                     task binds to; without one use /claim)"
            .to_string());
    }
    if req.get("count").and_then(|c| c.as_i64()).unwrap_or(1) > 1 {
        return Err("claim-flow: batch claims (--count > 1) bind one session per                     item — dispatch or send each item instead"
            .to_string());
    }
    let from_session = req
        .get("from_session")
        .and_then(|s| s.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if from_session {
        return Err(
            "claim-flow: from_session transfers keep the claim/lane/bind sequence".to_string(),
        );
    }
    Ok(session.to_string())
}

/// #5757-A: the operator-facing note for a /claim-flow partial
/// success — the claim COMMITTED and stands, but the lane or bind
/// step failed broker-side. The claimed task's identity is
/// preserved (no re-claim, no replay); the caller finishes the
/// named step with the single-verb call for this task id.
/// Returns None on a full success (or a non-claim-flow answer).
pub(super) fn claim_flow_partial_note(resp: &Value) -> Option<String> {
    let partial = resp.get("claim_flow_partial")?;
    let step = partial
        .get("step")
        .and_then(|s| s.as_str())
        .unwrap_or("lane/bind");
    let error = partial
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("unknown error");
    let task_id = resp
        .get("row")
        .and_then(|r| r.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    Some(format!(
        "repo-queue: claim-flow claimed task {task_id} but the {step} step failed ({error}); \
         the claim stands — finish that step with the single-verb call for task {task_id}, do not re-claim"
    ))
}

/// Whether this claim takes the one-trip /claim-flow path
/// (#5757-A): only with an explicit --bind, a single (non-batch)
/// claim, no --from-session transfer, and a --session-id to
/// bind. --bind is never implied by --session-id alone: existing
/// callers pass --session-id as a claim mark and keep
/// byte-identical behavior without the flag. Each refusal names
/// the flag at fault — a caller who asked for a bind must never
/// silently get an unbound claim. Mirrors claim_flow_precheck
/// (the broker-side enforcement of the same invariants).
pub(crate) fn claim_bind_path(
    bind: bool,
    count: i64,
    from_session: bool,
    session_id: Option<&str>,
) -> Result<bool, String> {
    if !bind {
        return Ok(false);
    }
    if count > 1 {
        return Err("claim --bind: batch claims (--count > 1) bind one session per                     item — dispatch or send each item instead"
            .to_string());
    }
    if from_session {
        return Err(
            "claim --bind: --from-session transfers keep the claim/lane/bind sequence".to_string(),
        );
    }
    match session_id {
        Some(s) if !s.is_empty() => Ok(true),
        _ => Err(
            "claim --bind: --session-id is required (the session the claimed task binds to)"
                .to_string(),
        ),
    }
}

/// #9046: append one timeline record (model_stamp / session_bound
/// / session_unbound) through the broker's generic /events-append
/// verb. Fail-soft by design: a timeline record must never fail
/// the operation it annotates.
pub(super) fn append_timeline_event(worker: &str, task_id: i64, kind: &str, detail: Value) {
    let resp = call(
        "/events-append",
        &json!({
            "worker_id": worker,
            "task_id": task_id,
            "kind": kind,
            "detail": detail,
        }),
    );
    if resp.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        eprint(&format!(
            "repo-queue: warning: {kind} timeline append for task {task_id} failed"
        ));
    }
}

/// #9046: the stamp a claim surface carries, as (provider, model,
/// variant) — read back off the request body cmd_claim built, so
/// explicit flags and session-derived stamps are indistinguishable
/// here (both were resolved before the claim was sent).
pub(super) fn body_stamp(body: &Value) -> Option<(String, String, Option<String>)> {
    let provider = body.get("model_provider")?.as_str()?.to_string();
    let model = body.get("model_id")?.as_str()?.to_string();
    let variant = body
        .get("model_variant")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some((provider, model, variant))
}

/// #9046: after a claim, mirror the binding + stamp into the
/// task's timeline when the broker did not write them itself.
/// Capability signal: a broker that stamps in-op (post-#9046)
/// returns claim_session_id on the claimed row (TASK_COLS carries
/// it); a pre-switch broker's row has no such key, and
/// /events-append is the live path. Post-switch this is a no-op.
pub(super) fn claim_timeline_appends(
    worker: &str,
    row: &Value,
    stamp: &Option<(String, String, Option<String>)>,
    session_id: &Option<String>,
    trigger: &str,
) {
    if row.get("claim_session_id").is_some() {
        return;
    }
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    if task_id == 0 {
        return;
    }
    if let Some(sid) = session_id {
        append_timeline_event(
            worker,
            task_id,
            "session_bound",
            json!({"session_id": sid, "trigger": trigger, "via": "fleet-cli"}),
        );
    }
    if let Some((provider, model, variant)) = stamp {
        append_timeline_event(
            worker,
            task_id,
            "model_stamp",
            json!({
                "session_id": session_id,
                "provider": provider,
                "model": model,
                "variant": variant,
                "trigger": trigger,
                "via": "fleet-cli",
            }),
        );
    }
}

/// #9046: mirror a binding close into the timeline when a
/// transition verb's answer row shows the task had a session
/// bound (progress.session_id, or the just-cleared
/// progress.session_cleared.session_id). A post-switch broker
/// writes session_unbound in-op; the timeline reader collapses
/// the duplicate (same kind/session/trigger inside 120 s).
pub(super) fn unbound_append(worker: &str, row: &Value, trigger: &str) {
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    if task_id == 0 {
        return;
    }
    let progress = row.get("progress").cloned().unwrap_or(Value::Null);
    let sid = progress
        .get("session_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            progress
                .get("session_cleared")
                .and_then(|c| c.get("session_id"))
                .and_then(|v| v.as_str())
        });
    if let Some(sid) = sid {
        append_timeline_event(
            worker,
            task_id,
            "session_unbound",
            json!({"session_id": sid, "trigger": trigger, "via": "fleet-cli"}),
        );
    }
}

/// #9046: fleet session ids are `ses_` + alphanumerics — the
/// shape every session surface mints. Anything else is not a
/// session id and must never reach a route_attempt SQL string.
/// #8287: this is the crate's one session-id predicate — the
/// broker's bind verb (turso_queue::verb_bind) validates against
/// it too; do not fork a second definition.
pub(crate) fn valid_session_id(s: &str) -> bool {
    s.len() > 4 && s.starts_with("ses_") && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// #9046: the OpenCode server's raw route ledger — the data the
/// session join reads (route proxy's OPENCODE_ROUTE_PROXY_DB).
pub(super) fn route_attempt_db() -> String {
    std::env::var("OPENCODE_ROUTE_PROXY_DB")
        .unwrap_or_else(|_| format!("{}/.local/share/opencode/opencode.db", home()))
}

/// #9046: the dominant serving leaf for one session over one
/// window, by route_attempt call count (the close-out summary).
/// Read-only sqlite3 shell-out; None on any failure — callers
/// proceed without the summary rather than blocking.
pub(super) fn dominant_model_for_session(
    session_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Option<Value> {
    if !valid_session_id(session_id) || end_ms <= start_ms {
        return None;
    }
    let sql = format!(
        "SELECT provider_id, model_id, variant, COUNT(*) AS c FROM route_attempt \
         WHERE session_id = '{session_id}' AND time_started >= {start_ms} \
         AND time_started < {end_ms} \
         GROUP BY provider_id, model_id, variant ORDER BY c DESC LIMIT 1"
    );
    let out = std::process::Command::new("sqlite3")
        .args(["-readonly", "-json", &route_attempt_db(), &sql])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rows: Value = serde_json::from_slice(&out.stdout).ok()?;
    let r = rows.as_array()?.first()?;
    Some(json!({
        "provider": r.get("provider_id")?,
        "model": r.get("model_id")?,
        "variant": r.get("variant").cloned().unwrap_or(Value::Null),
        "calls": r.get("c")?,
        "session_id": session_id,
        "window_start_ms": start_ms,
        "window_end_ms": end_ms,
        "source": "route_attempt",
    }))
}

/// #9046: the exact model segments one session served inside one
/// window, from route_attempt (provider/model/variant + call
/// count, chronological). None when the raw data is unreachable.
pub(super) fn route_segments_for_session(
    session_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Option<Vec<String>> {
    if !valid_session_id(session_id) || end_ms <= start_ms {
        return None;
    }
    let sql = format!(
        "SELECT provider_id, model_id, variant, COUNT(*) AS c FROM route_attempt \
         WHERE session_id = '{session_id}' AND time_started >= {start_ms} \
         AND time_started < {end_ms} \
         GROUP BY provider_id, model_id, variant ORDER BY MIN(time_started)"
    );
    let out = std::process::Command::new("sqlite3")
        .args(["-readonly", "-json", &route_attempt_db(), &sql])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rows: Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut segs = Vec::new();
    for r in rows.as_array()? {
        let variant = r
            .get("variant")
            .and_then(|v| v.as_str())
            .map(|v| format!("#{v}"))
            .unwrap_or_default();
        segs.push(format!(
            "session={} {}/{}{} x{}",
            session_id,
            r.get("provider_id")?.as_str()?,
            r.get("model_id")?.as_str()?,
            variant,
            r.get("c")?.as_i64()?,
        ));
    }
    Some(segs)
}

/// #9046: the close-out summary for a task being completed: its
/// bound session (progress.session_id) over its claim window
/// [claimed_at, now], resolved against route_attempt. Needs the
/// task's queue to fetch its row; every miss returns None and
/// the completion proceeds without the summary.
pub(super) fn dominant_model_for_task(args: &Args, task_id: i64) -> Option<Value> {
    let queue = opt_queue_scoped(args)?;
    let resp = call(
        "/list",
        &json!({"queue": queue, "rich": true, "task": task_id, "limit": 1}),
    );
    let row = resp.get("rows")?.as_array()?.first()?;
    let session = row.get("progress")?.get("session_id")?.as_str()?;
    let start_ms = parse_ts(row.get("claimed_at")?)? / 1000;
    let end_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    dominant_model_for_session(session, start_ms, end_ms)
}

/// #9046: one timeline record as the reader keeps it, normalized
/// over an /events-list row (pre-#9046 kinds fold in:
/// lane_session_bound is a bind, session_binding_cleared an
/// unbind; a claimed event's detail carries its session).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TimelineEvent {
    pub(super) id: i64,
    pub(super) kind: String,
    pub(super) session_id: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) trigger: Option<String>,
    pub(super) created_at: String,
    pub(super) created_us: i64,
}

pub(super) fn timeline_event(row: &Value) -> Option<TimelineEvent> {
    let detail = match row.get("detail") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let str_at = |key: &str| detail.get(key).and_then(|x| x.as_str()).map(str::to_string);
    let kind = row.get("kind")?.as_str()?;
    let kind = match kind {
        "lane_session_bound" => "session_bound",
        "session_binding_cleared" => "session_unbound",
        other => other,
    }
    .to_string();
    Some(TimelineEvent {
        id: row.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        kind,
        session_id: str_at("session_id"),
        provider: str_at("provider").or_else(|| str_at("model_provider")),
        model: str_at("model").or_else(|| str_at("model_id")),
        variant: str_at("variant").or_else(|| str_at("model_variant")),
        trigger: str_at("trigger").or_else(|| str_at("via")),
        created_at: row.get("created_at")?.as_str()?.to_string(),
        created_us: row.get("created_at").and_then(parse_ts).unwrap_or(0),
    })
}

/// #9046 oracle: collapse duplicate timeline writes — the same
/// (kind, session, provider, model, variant, trigger) recorded
/// twice within 120 s is one record written by two surfaces (a
/// pre-switch CLI append and the post-switch broker write of the
/// same touch), never two transitions. Anything else — a changed
/// field, or the same shape after the window — is a real second
/// record and stays.
pub(super) fn collapse_timeline(events: Vec<TimelineEvent>) -> Vec<TimelineEvent> {
    let mut out: Vec<TimelineEvent> = Vec::new();
    for ev in events {
        if let Some(prev) = out.last() {
            let same = prev.kind == ev.kind
                && prev.session_id == ev.session_id
                && prev.provider == ev.provider
                && prev.model == ev.model
                && prev.variant == ev.variant
                && prev.trigger == ev.trigger;
            if same && ev.created_us - prev.created_us <= 120_000_000 {
                continue;
            }
        }
        out.push(ev);
    }
    out
}

/// Keep the human event representation identical between `history`
/// and the bounded history shown by `show`.
pub(super) fn format_timeline_event(ev: &TimelineEvent) -> String {
    let mut line = format!("{}  {}", ev.created_at, ev.kind);
    if let Some(sid) = &ev.session_id {
        line += &format!("  session={sid}");
    }
    if let Some(provider) = &ev.provider {
        line += &format!("  {provider}/{}", ev.model.as_deref().unwrap_or("?"));
        if let Some(variant) = &ev.variant {
            line += &format!("#{variant}");
        }
    }
    if let Some(trigger) = &ev.trigger {
        line += &format!("  ({trigger})");
    }
    line
}

/// #9046 oracle: pair session_bound -> session_unbound per
/// session into windows (session, start, end-or-None), in the
/// order the windows close (open bindings last) — the windows
/// that slice the session's route_attempt rows per task.
pub(super) fn binding_windows(events: &[TimelineEvent]) -> Vec<(String, String, Option<String>)> {
    let mut open: Vec<(String, String)> = Vec::new();
    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    for ev in events {
        match ev.kind.as_str() {
            "session_bound" => {
                if let Some(sid) = &ev.session_id {
                    open.push((sid.clone(), ev.created_at.clone()));
                }
            }
            "session_unbound" => {
                if let Some(sid) = &ev.session_id {
                    if let Some(pos) = open.iter().position(|(s, _)| s == sid) {
                        let (s, start) = open.remove(pos);
                        out.push((s, start, Some(ev.created_at.clone())));
                    }
                }
            }
            _ => {}
        }
    }
    for (s, start) in open {
        out.push((s, start, None));
    }
    out
}

/// #9046: render one task's model timeline — binding windows,
/// stamps, and transitions in order, with the exact resolved
/// models per window joined from route_attempt.
pub(super) fn cmd_timeline(verb: &str, rest: &[String]) -> i32 {
    let args = parse_verb_args(verb, rest, &["worker"], &["json"]);
    reject_extra_pos(verb, &args, 1);
    let task_id = args.pos_i64(0, "task_id");
    let mut events: Vec<TimelineEvent> = Vec::new();
    for kind in [
        "claimed",
        "staleness_autocomplete",
        "session_bound",
        "lane_session_bound",
        "model_stamp",
        "session_unbound",
        "session_binding_cleared",
        "completed",
        "lease_expired",
        "lease_expired_at_cap",
        "claim_refused",
        "released",
        "parked",
        "reprioritized",
        "reprioritize_sync_pending",
        "failed",
        "cancelled",
    ] {
        let resp = call(
            "/events-list",
            &json!({"task_id": task_id, "kind": kind, "limit": 1000}),
        );
        if resp.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            eprint(
                "repo-queue timeline: this broker does not serve /events-list yet \
                 — the full timeline activates with the #9046 broker at the \
                 coordinated switch. Interim read (fleet-queue DB): SELECT id, \
                 kind, detail, created_at FROM events WHERE task_id = <id> AND \
                 kind IN ('model_stamp','session_bound','session_unbound',\
                 'lane_session_bound','session_binding_cleared') ORDER BY id;",
            );
            return 3;
        }
        if let Some(rows) = resp
            .get("row")
            .and_then(|r| r.get("rows"))
            .and_then(|r| r.as_array())
        {
            for row in rows {
                if let Some(ev) = timeline_event(row) {
                    events.push(ev);
                }
            }
        }
    }
    events.sort_by_key(|e| e.id);
    let events = collapse_timeline(events);
    // #14170 Q4: the task's notes interleave with its events into
    // one chronological story. Fail-soft: a broker without
    // /note-list renders the events alone.
    let (notes, _) = fetch_notes(task_id, None).unwrap_or_default();
    let json_out = args.has("json");
    if events.is_empty() && notes.is_empty() {
        if json_out {
            print_json(
                &json!({"task_id": task_id, "events": [], "notes": [], "story": [], "served": []}),
            );
            return 0;
        }
        println!("task {task_id}: no timeline records");
        return 0;
    }
    if json_out {
        // --json (#8200 residual): the same records the table below
        // prints, as structured data — events in order, plus the
        // route_attempt served windows. Marks print as recorded:
        // an absent mark is null, never invented.
        let windows = binding_windows(&events);
        let served: Vec<Value> = windows
            .iter()
            .map(|(sid, start, end)| {
                let start_ms = parse_ts_str(start).map(|us| us / 1000).unwrap_or(0);
                let end_ms = end
                    .as_ref()
                    .and_then(|e| parse_ts_str(e))
                    .map(|us| us / 1000)
                    .unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0)
                    });
                json!({
                    "session_id": sid,
                    "start": start,
                    "end": end,
                    "segments": route_segments_for_session(sid, start_ms, end_ms)
                        .unwrap_or_default(),
                })
            })
            .collect();
        print_json(&json!({
            "task_id": task_id,
            "events": events
                .iter()
                .map(|ev| {
                    json!({
                        "id": ev.id,
                        "kind": ev.kind,
                        "created_at": ev.created_at,
                        "session_id": ev.session_id,
                        "provider": ev.provider,
                        "model": ev.model,
                        "variant": ev.variant,
                        "trigger": ev.trigger,
                    })
                })
                .collect::<Vec<_>>(),
            "notes": notes.iter().map(note_json).collect::<Vec<_>>(),
            "story": interleave_notes(&events, &notes)
                .iter()
                .map(|entry| match entry {
                    TimelineEntry::Event(ev) => json!({"type": "event", "id": ev.id, "created_at": ev.created_at}),
                    TimelineEntry::Note(n) => json!({"type": "note", "id": n.id, "created_at": n.created_at}),
                })
                .collect::<Vec<_>>(),
            "served": served,
        }));
        return 0;
    }
    for entry in interleave_notes(&events, &notes) {
        match entry {
            TimelineEntry::Event(ev) => println!("{}", format_timeline_event(ev)),
            TimelineEntry::Note(n) => println!("{}", format_timeline_note(n)),
        }
    }
    // The join: exact resolved models per binding window, from
    // the session's raw route_attempt rows (laptop-local,
    // fail-soft — unreachable route data says so and moves on).
    let windows = binding_windows(&events);
    if !windows.is_empty() {
        println!("served (route_attempt join):");
        for (sid, start, end) in &windows {
            let start_ms = parse_ts_str(start).map(|us| us / 1000).unwrap_or(0);
            let end_ms = end
                .as_ref()
                .and_then(|e| parse_ts_str(e))
                .map(|us| us / 1000)
                .unwrap_or_else(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0)
                });
            match route_segments_for_session(sid, start_ms, end_ms) {
                Some(segs) if !segs.is_empty() => {
                    for seg in segs {
                        println!("  {seg}");
                    }
                }
                _ => println!("  session={sid}: no route attempts in window"),
            }
        }
    }
    0
}

/// #10357 (claimant capture point) + #9046: the per-model
/// rollup — the inversion of cmd_timeline. The link records
/// (model_stamp / session_bound events) already land per task;
/// this verb aggregates them across tasks over a window so
/// per-model accounting — which models worked which tasks, with
/// what outcome — is one query instead of hand-rolled SQL.
/// Two sections: `stamped` (the claim/bind stamp timeline: the
/// route or lane a task was claimed under) and `served` (the
/// dominant serving leaf per binding window, from the same
/// route_attempt join cmd_timeline prints — for routed stamps
/// this is the model that actually did the work).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RollupEvent {
    pub(super) task_id: i64,
    pub(super) ev: TimelineEvent,
}

pub(super) fn rollup_event(row: &Value) -> Option<RollupEvent> {
    let task_id = row.get("task_id").and_then(|v| v.as_i64())?;
    let ev = timeline_event(row)?;
    Some(RollupEvent { task_id, ev })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ModelKey {
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) variant: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct ModelRollup {
    pub(super) tasks: std::collections::BTreeSet<i64>,
    pub(super) sessions: std::collections::BTreeSet<String>,
    pub(super) stamps: usize,
    pub(super) completed: usize,
}

/// The stamped section: one rollup per (provider, model,
/// variant) over model_stamp records (already duplicate-
/// collapsed per task by the caller). The completion count
/// intersects the stamped task set with the window's completed
/// set — a task counts once per model however many stamps it
/// carries. (No per-model PR count: the store's pull_request
/// events are a GitHub webhook mirror whose task_id is always
/// NULL, and merge_intents link PRs to issues/sessions, not
/// tasks — there is no task-linked PR signal to count yet.)
pub(super) fn rollup_models(
    events: &[RollupEvent],
    completed: &std::collections::BTreeSet<i64>,
) -> Vec<(ModelKey, ModelRollup)> {
    let mut map: std::collections::BTreeMap<ModelKey, ModelRollup> =
        std::collections::BTreeMap::new();
    for re in events {
        if re.ev.kind != "model_stamp" {
            continue;
        }
        let (Some(p), Some(m)) = (&re.ev.provider, &re.ev.model) else {
            continue;
        };
        let key = ModelKey {
            provider: p.clone(),
            model: m.clone(),
            variant: re.ev.variant.clone(),
        };
        let r = map.entry(key).or_default();
        r.tasks.insert(re.task_id);
        if let Some(sid) = &re.ev.session_id {
            r.sessions.insert(sid.clone());
        }
        r.stamps += 1;
    }
    for r in map.values_mut() {
        r.completed = r.tasks.iter().filter(|t| completed.contains(t)).count();
    }
    map.into_iter().collect()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct ServedRollup {
    pub(super) tasks: std::collections::BTreeSet<i64>,
    pub(super) windows: usize,
    pub(super) calls: i64,
}

/// The served section: one rollup per dominant serving leaf,
/// over (task, leaf, route_attempt calls) rows the caller
/// resolved per binding window.
pub(super) fn rollup_served(rows: &[(i64, ModelKey, i64)]) -> Vec<(ModelKey, ServedRollup)> {
    let mut map: std::collections::BTreeMap<ModelKey, ServedRollup> =
        std::collections::BTreeMap::new();
    for (task_id, key, calls) in rows {
        let r = map.entry(key.clone()).or_default();
        r.tasks.insert(*task_id);
        r.windows += 1;
        r.calls += calls;
    }
    map.into_iter().collect()
}

/// Format a unix-seconds instant in the ISO-8601 UTC shape
/// the events store writes (created_at compares
/// lexicographically, so the since floor is a text compare).
pub(super) fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60
    )
}

/// `repo-queue models [--days N] [--json]` — the claim-link
/// rollup (#10357): the inversion of `timeline`. timeline
/// answers "which models worked task X" per task; this answers
/// "which tasks did model Y touch" across the fleet, over the
/// link #9046 records (model_stamp / session_bound events in
/// the queue store, joined to route_attempt by binding window).
/// Two sections: stamped — per (provider, model, variant) the
/// distinct tasks + sessions its stamps touch and how many of
/// those tasks completed in the window; served — per dominant
/// serving leaf (the route actually used inside each binding
/// window) the tasks, windows and provider calls. Needs the
/// broker build whose /events-list makes task_id optional;
/// against an older broker it says so and exits 3. The window
/// clamps to the store's 30-day event retention.
pub(super) fn cmd_models(rest: &[String]) -> i32 {
    let args = parse_verb_args("models", rest, &["days"], &["json"]);
    reject_extra_pos("models", &args, 0);
    let days = args.get_i64("days").unwrap_or(7).clamp(1, 30);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let since = iso_utc(now_secs - days * 86_400);
    // Fetch the window's link + outcome events across tasks.
    let mut by_task: std::collections::BTreeMap<i64, Vec<TimelineEvent>> =
        std::collections::BTreeMap::new();
    let mut completed: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    for kind in [
        "model_stamp",
        "session_bound",
        "lane_session_bound",
        "lane_session_unbound",
        "session_unbound",
        "session_binding_cleared",
        "completed",
    ] {
        let resp = call(
            "/events-list",
            &json!({"kind": kind, "since": since, "limit": 50_000}),
        );
        // The broker answers /events-list with a flat
        // {"events": [...]} (the shape cmd_timeline reads). Any
        // other envelope — an old broker's error for the
        // task-less call, say — means the widened read is not
        // live yet; say so instead of printing an empty rollup.
        let Some(rows) = resp.get("events").and_then(|r| r.as_array()).cloned() else {
            eprint(
                "repo-queue models: this broker does not serve cross-task \
                 /events-list yet — the rollup activates with the broker \
                 build that makes task_id optional (interim: repo-queue \
                 timeline <task> answers per task).",
            );
            return 3;
        };
        if rows.len() >= 50_000 {
            eprint(&format!(
                "repo-queue models: warning: {kind} hit the 50000-row \
                 window cap; its counts are truncated\n"
            ));
        }
        for row in &rows {
            let Some(task_id) = row.get("task_id").and_then(|v| v.as_i64()) else {
                continue;
            };
            match kind {
                "completed" => {
                    completed.insert(task_id);
                }
                _ => {
                    if let Some(re) = rollup_event(row) {
                        by_task.entry(re.task_id).or_default().push(re.ev);
                    }
                }
            }
        }
    }
    // Collapse each task's timeline (the #9046 duplicate
    // oracle) before aggregating, so a touch written by two
    // surfaces counts once; resolve each binding window's
    // dominant serving leaf on the way.
    let mut flat: Vec<RollupEvent> = Vec::new();
    let mut served_rows: Vec<(i64, ModelKey, i64)> = Vec::new();
    let now_ms = now_secs * 1000;
    for (task_id, mut evs) in by_task {
        evs.sort_by_key(|e| e.id);
        let evs = collapse_timeline(evs);
        for (sid, start, end) in binding_windows(&evs) {
            let start_ms = parse_ts_str(&start).map(|us| us / 1000).unwrap_or(0);
            let end_ms = end
                .as_ref()
                .and_then(|e| parse_ts_str(e))
                .map(|us| us / 1000)
                .unwrap_or(now_ms);
            if let Some(dom) = dominant_model_for_session(&sid, start_ms, end_ms) {
                let key = ModelKey {
                    provider: dom
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    model: dom
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    variant: dom
                        .get("variant")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                };
                let calls = dom.get("calls").and_then(|v| v.as_i64()).unwrap_or(0);
                served_rows.push((task_id, key, calls));
            }
        }
        flat.extend(evs.into_iter().map(|ev| RollupEvent { task_id, ev }));
    }
    let stamped = rollup_models(&flat, &completed);
    let served = rollup_served(&served_rows);
    let disp = |k: &ModelKey| {
        let variant = k
            .variant
            .as_ref()
            .map(|v| format!("#{v}"))
            .unwrap_or_default();
        format!("{}/{}{}", k.provider, k.model, variant)
    };
    if args.has("json") {
        let key_json =
            |k: &ModelKey| json!({"provider": k.provider, "model": k.model, "variant": k.variant});
        print_json(&json!({
            "since": since,
            "days": days,
            "stamped": stamped.iter().map(|(k, r)| {
                let mut o = key_json(k);
                o["tasks"] = json!(r.tasks.len());
                o["task_ids"] = json!(r.tasks.iter().collect::<Vec<_>>());
                o["sessions"] = json!(r.sessions.len());
                o["stamps"] = json!(r.stamps);
                o["completed"] = json!(r.completed);
                o
            }).collect::<Vec<_>>(),
            "served": served.iter().map(|(k, r)| {
                let mut o = key_json(k);
                o["tasks"] = json!(r.tasks.len());
                o["task_ids"] = json!(r.tasks.iter().collect::<Vec<_>>());
                o["windows"] = json!(r.windows);
                o["calls"] = json!(r.calls);
                o
            }).collect::<Vec<_>>(),
        }));
        return 0;
    }
    println!("model rollup — last {days}d (since {since})");
    println!("stamped (claim/bind stamps):");
    if stamped.is_empty() {
        println!("  (none)");
    }
    for (k, r) in &stamped {
        println!(
            "  {}  tasks={} sessions={} stamps={} completed={}",
            disp(k),
            r.tasks.len(),
            r.sessions.len(),
            r.stamps,
            r.completed
        );
    }
    println!("served (dominant leaf per binding window):");
    if served.is_empty() {
        println!("  (none)");
    }
    for (k, r) in &served {
        println!(
            "  {}  tasks={} windows={} calls={}",
            disp(k),
            r.tasks.len(),
            r.windows,
            r.calls
        );
    }
    0
}

/// Whether a broker/transport error message names an access
/// denial, as opposed to an unavailable or failing store (#8581
/// pilot: a denial is reported as its own detail — it is never
/// retried under another identity and never reads as absence).
fn denial_shaped(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["unauthorized", "denied", "forbidden", "permission"]
        .iter()
        .any(|w| m.contains(w))
}

/// Stamp the outcome triple (mode, route, result) on the pilot
/// record.
fn exact_outcome(record: &mut Value, mode: &str, route: &str, result: &str) {
    record["mode"] = json!(mode);
    record["route"] = json!(route);
    record["result"] = json!(result);
}

/// #8581 exact-task status pilot (read-only first slice): the
/// dispatcher for `list --json`. Both selectors are required —
/// the JSON form is defined as the exact read, never as a
/// rendering of the bulk listing — and the bulk narrowing flags
/// do not apply to it: the one scoped read is named by queue +
/// task alone, and a silently ignored flag would misstate what
/// was asked. Exactly one broker read follows; the verb prints
/// the single record [`exact_task_status`] builds and returns
/// its exit code. Nothing here writes.
fn cmd_list_exact_status(args: &Args, queue: Option<String>) -> i32 {
    let Some(queue) = queue else {
        usage_error(
            "list",
            "--json exact status requires --queue <name> and --task <id>",
        );
    };
    let Some(task_id) = args.get_i64("task") else {
        usage_error(
            "list",
            "--json exact status requires --queue <name> and --task <id>",
        );
    };
    for flag in ["status", "limit", "not-before", "lane", "worker"] {
        if args.get(flag).is_some() {
            usage_error(
                "list",
                &format!(
                    "--json exact status takes only --queue and --task; --{flag} does not apply"
                ),
            );
        }
    }
    let resp = call_result(
        "/list",
        &json!({"rich": true, "queue": queue, "task": task_id, "limit": 1}),
    );
    let (record, code) = exact_task_status(&queue, task_id, resp, &utcnow_iso());
    print_json(&record);
    code
}

/// Pure core of the #8581 exact-task status pilot: fold the one
/// scoped /list result into the single emitted record and the
/// verb's exit code.
///
/// The record names the verb, the mode and the route actually
/// used (the #8581 contract vocabulary, via the mode adapter),
/// and one of four results:
///
/// - `found` — the authoritative reply carries the exact task;
///   its broker row rides under `task`, attributed to `source`
///   with the row's own `task_as_of` when the row carries a
///   timestamp. Exit 0.
/// - `not_found` — ONLY a complete authoritative zero: an ok
///   reply with an empty rows array, a reported total of 0, and
///   no truncation. Exit 1 (the client's not-found class, the
///   one `show` uses).
/// - `partial` — a reply arrived but cannot certify the answer:
///   a window smaller than the matching set (`truncated_reply`),
///   a zero whose total was never reported
///   (`total_unreported`), or a complete reply naming other
///   tasks — the broker did not scope the read
///   (`unscoped_reply`). Exit 3.
/// - `unknown` — nothing trustworthy was learned: the transport
///   never produced an envelope (`unavailable`), access was
///   denied (`denied`), the broker returned an error envelope
///   (`broker_error`), the reply was malformed
///   (`malformed_reply`), or the matching row names a different
///   queue than the one asked about (`identity_mismatch`).
///   Exit 3, or the broker envelope's own exit for
///   `broker_error` (the unwrap parity the text verbs use).
///
/// No outcome other than the certified zero is ever reported as
/// absence. When no route served, the record says so plainly:
/// mode `db-degraded`, route `unavailable` — this slice attempts
/// no replica or marker fallback, so it names none.
pub(crate) fn exact_task_status(
    queue: &str,
    task_id: i64,
    resp: Result<Value, (i32, String)>,
    observed_at: &str,
) -> (Value, i32) {
    let mut record = json!({
        "verb": "list",
        "queue": queue,
        "task_id": task_id,
        "as_of": observed_at,
    });
    match resp {
        Err((_code, msg)) => {
            // The transport produced no envelope: the broker path
            // is unavailable, or the token was rejected at the
            // proxy leg. Nothing was read.
            let detail = if denial_shaped(&msg) {
                "denied"
            } else {
                "unavailable"
            };
            exact_outcome(
                &mut record,
                crate::work::degraded::Mode::DbDegraded.label(),
                "unavailable",
                "unknown",
            );
            record["detail"] = json!(detail);
            record["error"] = json!(msg);
            (record, 3)
        }
        Ok(env) if env.get("ok").and_then(Value::as_bool) == Some(false) => {
            let msg = env
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("queue-broker error")
                .to_string();
            let code = env.get("exit").and_then(Value::as_i64).unwrap_or(4) as i32;
            let detail = if denial_shaped(&msg) {
                "denied"
            } else {
                "broker_error"
            };
            exact_outcome(
                &mut record,
                crate::work::degraded::Mode::DbDegraded.label(),
                "unavailable",
                "unknown",
            );
            record["detail"] = json!(detail);
            record["error"] = json!(msg);
            (record, code)
        }
        Ok(env) => {
            // The broker answered: the cloud route served.
            let line = mode_line_for("list", legs_served(true, true));
            let (mode, route) = (line.mode.label(), line.route);
            record["source"] = json!("cloud");
            if env.get("ok").and_then(Value::as_bool) != Some(true) {
                exact_outcome(&mut record, mode, route, "unknown");
                record["detail"] = json!("malformed_reply");
                record["error"] = json!(
                    "queue-broker /list reply carried no ok flag — refusing to read it as an answer"
                );
                return (record, 3);
            }
            let rows = match list_rows(&env) {
                Ok(rows) => rows,
                Err(e) => {
                    exact_outcome(&mut record, mode, route, "unknown");
                    record["detail"] = json!("malformed_reply");
                    record["error"] = json!(e);
                    return (record, 3);
                }
            };
            if let Some(row) = rows
                .iter()
                .find(|r| r.get("id").and_then(Value::as_i64) == Some(task_id))
            {
                if let Some(row_queue) = row.get("queue").and_then(Value::as_str) {
                    if row_queue != queue {
                        exact_outcome(&mut record, mode, route, "unknown");
                        record["detail"] = json!("identity_mismatch");
                        record["error"] = json!(format!(
                            "the matching row names queue {row_queue:?}, not the requested {queue:?}"
                        ));
                        return (record, 3);
                    }
                }
                exact_outcome(&mut record, mode, route, "found");
                if let Some(ts) = row
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .or_else(|| row.get("created_at").and_then(Value::as_str))
                {
                    record["task_as_of"] = json!(ts);
                }
                record["task"] = row.clone();
                return (record, 0);
            }
            let total = list_total(&env);
            let truncated = env
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if rows.is_empty() && total == Some(0) && !truncated {
                exact_outcome(&mut record, mode, route, "not_found");
                return (record, 1);
            }
            let detail = if truncated || total.map(|t| t > rows.len() as i64).unwrap_or(false) {
                "truncated_reply"
            } else if rows.is_empty() && total.is_none() {
                "total_unreported"
            } else {
                "unscoped_reply"
            };
            exact_outcome(&mut record, mode, route, "partial");
            record["detail"] = json!(detail);
            (record, 3)
        }
    }
}

/// #14170 Q4: one task note as the timeline and `note list` render
/// it. Marks print as recorded: an absent mark is None.
pub(super) struct TimelineNote {
    pub(super) id: i64,
    pub(super) author: String,
    pub(super) author_kind: String,
    pub(super) session_id: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) body: String,
    pub(super) created_at: String,
    pub(super) created_us: i64,
    pub(super) edited_at: Option<String>,
    pub(super) deleted_at: Option<String>,
}

pub(super) fn timeline_note(row: &Value) -> Option<TimelineNote> {
    let s = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Some(TimelineNote {
        id: row.get("id").and_then(Value::as_i64)?,
        author: s("author").unwrap_or_default(),
        author_kind: s("author_kind").unwrap_or_else(|| "agent".to_string()),
        session_id: s("session_id"),
        provider: s("model_provider"),
        model: s("model_id"),
        variant: s("model_variant"),
        body: s("body").unwrap_or_default(),
        created_at: s("created_at")?,
        created_us: row.get("created_at").and_then(parse_ts).unwrap_or(0),
        edited_at: s("edited_at"),
        deleted_at: s("deleted_at"),
    })
}

pub(super) fn note_json(n: &TimelineNote) -> Value {
    json!({
        "id": n.id, "author": n.author, "author_kind": n.author_kind,
        "session_id": n.session_id, "provider": n.provider, "model": n.model,
        "variant": n.variant, "body": n.body, "created_at": n.created_at,
        "edited_at": n.edited_at, "deleted_at": n.deleted_at,
    })
}

/// A note's line in the same shape as an event line: timestamp,
/// `note #id`, author and marks, then the body indented beneath.
pub(super) fn format_timeline_note(n: &TimelineNote) -> String {
    let mut line = format!("{}  note #{}  by {}", n.created_at, n.id, n.author);
    if n.author_kind != "agent" {
        line += &format!(" ({})", n.author_kind);
    }
    if let Some(sid) = &n.session_id {
        line += &format!("  session={sid}");
    }
    if let Some(provider) = &n.provider {
        line += &format!("  {provider}/{}", n.model.as_deref().unwrap_or("?"));
        if let Some(variant) = &n.variant {
            line += &format!("#{variant}");
        }
    }
    if let Some(at) = &n.edited_at {
        line += &format!("  (edited {at})");
    }
    if let Some(at) = &n.deleted_at {
        line += &format!("  (removed {at})");
    }
    for body_line in n.body.lines() {
        line += &format!("\n    {body_line}");
    }
    line
}

/// One entry of a task's story: a history event or a note.
pub(super) enum TimelineEntry<'a> {
    Event(&'a TimelineEvent),
    Note(&'a TimelineNote),
}

/// #14170 Q4: interleave notes into the (already ordered and
/// collapsed) events by timestamp, so a reader sees one
/// chronological story. Each input keeps its own order; on a
/// timestamp tie the event comes first (the store stamps whole
/// seconds, and a note about a transition is written after it).
pub(super) fn interleave_notes<'a>(
    events: &'a [TimelineEvent],
    notes: &'a [TimelineNote],
) -> Vec<TimelineEntry<'a>> {
    let mut out = Vec::with_capacity(events.len() + notes.len());
    let (mut i, mut j) = (0, 0);
    while i < events.len() || j < notes.len() {
        let take_event = match (events.get(i), notes.get(j)) {
            (Some(e), Some(n)) => e.created_us <= n.created_us,
            (Some(_), None) => true,
            _ => false,
        };
        if take_event {
            out.push(TimelineEntry::Event(&events[i]));
            i += 1;
        } else {
            out.push(TimelineEntry::Note(&notes[j]));
            j += 1;
        }
    }
    out
}

/// A task's notes, oldest first, and its live note count. Fail-soft:
/// a broker that does not serve /note-list yet yields None, and the
/// caller renders the events alone.
pub(super) fn fetch_notes(task_id: i64, latest: Option<i64>) -> Option<(Vec<TimelineNote>, i64)> {
    let mut req = json!({"task_id": task_id});
    if let Some(n) = latest {
        req["latest"] = json!(n);
    }
    let resp = call("/note-list", &req);
    if resp.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let row = resp.get("row")?;
    let notes = row
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(timeline_note).collect())
        .unwrap_or_default();
    let count = row.get("count").and_then(Value::as_i64).unwrap_or(0);
    Some((notes, count))
}

/// `repo-queue note <add|list|edit|remove>` (#14170 Q4): the task
/// notes stream.
pub(super) fn cmd_note(rest: &[String]) -> i32 {
    if rest.is_empty() || is_help_arg(&rest[0]) {
        println!("{}", usage("note"));
        return if rest.is_empty() { 2 } else { 0 };
    }
    let tail = &rest[1..];
    match rest[0].as_str() {
        "add" => cmd_note_add(tail),
        "list" => cmd_note_list(tail),
        "edit" => cmd_note_edit(tail),
        "remove" => cmd_note_remove(tail),
        sub => {
            eprint(&format!(
                "repo-queue: unknown note command {sub:?}\n{}",
                usage("note")
            ));
            2
        }
    }
}

fn cmd_note_add(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "note add",
        rest,
        &[
            "worker",
            "text",
            "kind",
            "session-id",
            "model-provider",
            "model-id",
            "model-variant",
        ],
        &[],
    );
    reject_extra_pos("note add", &args, 1);
    let text = args
        .get("text")
        .unwrap_or_else(|| die(2, "note add: --text is required"));
    let body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "body": text,
        "author_kind": args.get("kind"),
        "session_id": args.get("session-id"),
        "model_provider": args.get("model-provider"),
        "model_id": args.get("model-id"),
        "model_variant": args.get("model-variant"),
    });
    let resp = unwrap_ok(call("/note-add", &body));
    print_json(&require_row(&resp, "note_add"));
    0
}

fn cmd_note_list(rest: &[String]) -> i32 {
    let args = parse_verb_args("note list", rest, &["latest"], &["include-removed", "json"]);
    reject_extra_pos("note list", &args, 1);
    let task_id = args.pos_i64(0, "task_id");
    let mut req = json!({"task_id": task_id, "include_removed": args.has("include-removed")});
    if let Some(n) = args.get_i64("latest") {
        req["latest"] = json!(n);
    }
    let resp = unwrap_ok(call("/note-list", &req));
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    let notes: Vec<TimelineNote> = row
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(timeline_note).collect())
        .unwrap_or_default();
    let count = row.get("count").and_then(Value::as_i64).unwrap_or(0);
    if args.has("json") {
        print_json(&json!({
            "task_id": task_id,
            "count": count,
            "notes": notes.iter().map(note_json).collect::<Vec<_>>(),
        }));
        return 0;
    }
    if notes.is_empty() {
        println!("task {task_id}: no notes");
        return 0;
    }
    println!("task {task_id}: {} of {count} notes", notes.len());
    for n in &notes {
        println!("{}", format_timeline_note(n));
    }
    0
}

fn cmd_note_edit(rest: &[String]) -> i32 {
    let args = parse_verb_args("note edit", rest, &["worker", "text"], &[]);
    reject_extra_pos("note edit", &args, 1);
    let text = args
        .get("text")
        .unwrap_or_else(|| die(2, "note edit: --text is required"));
    let body = json!({
        "note_id": args.pos_i64(0, "note_id"),
        "worker": opt_worker(&args),
        "body": text,
    });
    let resp = unwrap_ok(call("/note-edit", &body));
    print_json(&require_row(&resp, "note_edit"));
    0
}

fn cmd_note_remove(rest: &[String]) -> i32 {
    let args = parse_verb_args("note remove", rest, &["worker"], &[]);
    reject_extra_pos("note remove", &args, 1);
    let body = json!({
        "note_id": args.pos_i64(0, "note_id"),
        "worker": opt_worker(&args),
    });
    let resp = unwrap_ok(call("/note-remove", &body));
    print_json(&require_row(&resp, "note_remove"));
    0
}
