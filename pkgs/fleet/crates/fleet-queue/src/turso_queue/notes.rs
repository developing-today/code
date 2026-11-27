//! turso_queue::notes — the append-only `task_notes` stream (#14170 Q4).
//!
//! Notes are a stream (many, ordered, authored, timestamped), not
//! current state, so they get their own table rather than a metadata
//! key: two concurrent writers each append a row and neither can
//! overwrite the other (design §4).
//!
//! - `/note-add`: append a note. The author is the authenticated
//!   caller identity, the worker presented over the broker's
//!   X-Agent-Token channel, exactly like the lifecycle verbs. The
//!   session/model marks come from the request; when the author
//!   holds the task's claim and the request names none, the claim's
//!   own marks are copied at write time.
//! - `/note-list`: a task's notes in order, with the live count.
//! - `/note-edit`: author-only. The superseded body is preserved in
//!   the `note_edited` history row's detail (`previous_body`).
//! - `/note-remove`: author-only soft delete (`deleted_at`). Rows are
//!   never hard-deleted by a verb.
//!
//! Each write commits its note change, a `note_*` event and the
//! state-neutral task_history row (from = to = NULL) in one
//! BEGIN IMMEDIATE batch. The note kinds are deliberately absent
//! from `history_transition`: a generic `/events-append` writer
//! cannot fabricate a note's history row; only these verbs write
//! one, alongside the note itself.

use super::*;

/// Design §4: a note body is at most 4096 characters.
pub(crate) const NOTE_BODY_MAX_CHARS: usize = 4096;

/// The authors a note may name (design §4's author_kind).
pub(crate) const NOTE_AUTHOR_KINDS: [&str; 3] = ["agent", "human", "system"];

const NOTE_COLS: &str = "id, task_id, author, author_kind, session_id, model_provider, \
    model_id, model_variant, body, created_at, edited_at, deleted_at";

fn note_author(req: &Value) -> Result<String, String> {
    req_str(req, "worker")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "note: worker is required (the note's author)".to_string())
}

/// Validate a note body: non-empty after trimming, at most
/// NOTE_BODY_MAX_CHARS characters. The body is stored as given.
pub(crate) fn validate_note_body(req: &Value) -> Result<String, String> {
    let body = req_str(req, "body").unwrap_or("");
    if body.trim().is_empty() {
        return Err("note: body is required and must not be empty".to_string());
    }
    let n = body.chars().count();
    if n > NOTE_BODY_MAX_CHARS {
        return Err(format!(
            "note: body is {n} characters, over the {NOTE_BODY_MAX_CHARS}-character limit"
        ));
    }
    Ok(body.to_string())
}

fn get_note(t: &TursoBackend, note_id: i64) -> Result<Option<Value>, TErr> {
    Ok(t.query_objs(
        &format!("SELECT {NOTE_COLS} FROM task_notes WHERE id = ?"),
        &[Arg::Int(note_id)],
        &[],
    )?
    .into_iter()
    .next())
}

fn opt_text(req: &Value, key: &str) -> Option<String> {
    req_str(req, key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `/note-add`: append one note to a task.
pub(crate) fn verb_note_add(t: &TursoBackend, req: &Value) -> Value {
    let task_id = match req_i64(req, "task_id") {
        Some(id) => id,
        None => return fail(2, "note-add: task_id is required"),
    };
    let author = match note_author(req) {
        Ok(a) => a,
        Err(e) => return fail(2, e),
    };
    let body = match validate_note_body(req) {
        Ok(b) => b,
        Err(e) => return fail(2, e),
    };
    let author_kind = opt_text(req, "author_kind").unwrap_or_else(|| "agent".to_string());
    if !NOTE_AUTHOR_KINDS.contains(&author_kind.as_str()) {
        return fail(
            2,
            format!(
                "note-add: author_kind {author_kind:?} is not one of {}",
                NOTE_AUTHOR_KINDS.join(", ")
            ),
        );
    }
    let work = (|| -> Result<Value, String> {
        let task = get_task(t, task_id)?
            .ok_or_else(|| format!("note-add: task {task_id} does not exist"))?;
        // Marks: the request's own, else the claim's when the author
        // is the claim holder, copied now (never re-read later).
        let holds_claim = task.get("worker_id").and_then(Value::as_str) == Some(author.as_str());
        let claim_mark = |key: &str| -> Option<String> {
            if !holds_claim {
                return None;
            }
            let v = match key {
                "session_id" => task
                    .get("claim_session_id")
                    .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                    .or_else(|| task.get("progress").and_then(|p| p.get("session_id"))),
                other => task.get(other),
            };
            v.and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let session_id = opt_text(req, "session_id").or_else(|| claim_mark("session_id"));
        let provider = opt_text(req, "model_provider").or_else(|| claim_mark("claim_provider"));
        let model = opt_text(req, "model_id").or_else(|| claim_mark("claim_model"));
        let variant = opt_text(req, "model_variant").or_else(|| claim_mark("claim_variant"));
        let now = now_iso();
        let detail = json!({
            "actor": author, "author_kind": author_kind,
            "session_id": session_id, "model_provider": provider,
            "model_id": model, "model_variant": variant,
            "chars": body.chars().count(),
        });
        let results = t.exec_batch(&[
            ("BEGIN IMMEDIATE", vec![]),
            (
                "INSERT INTO task_notes (task_id, author, author_kind, session_id, \
                 model_provider, model_id, model_variant, body, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Arg::Int(task_id),
                    Arg::Text(author.clone()),
                    Arg::Text(author_kind.clone()),
                    arg_str(session_id.as_deref()),
                    arg_str(provider.as_deref()),
                    arg_str(model.as_deref()),
                    arg_str(variant.as_deref()),
                    Arg::Text(body.clone()),
                    Arg::Text(now.clone()),
                ],
            ),
            // The pointer: the note id is the row just inserted.
            (
                "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
                 SELECT ?, ?, 'note_added', json_set(?, '$.note_id', last_insert_rowid()), ? \
                 WHERE changes() = 1",
                vec![
                    Arg::Text(author.clone()),
                    Arg::Int(task_id),
                    Arg::Text(detail.to_string()),
                    Arg::Text(now.clone()),
                ],
            ),
            (HISTORY_INSERT_SQL, vec![Arg::Null, Arg::Null]),
            ("COMMIT", vec![]),
        ])?;
        let note_id = results
            .get(1)
            .filter(|r| r.affected == 1)
            .and_then(|r| r.last_insert_rowid)
            .ok_or_else(|| format!("note-add: the note on task {task_id} was not written"))?;
        get_note(t, note_id)?
            .ok_or_else(|| format!("note-add: note {note_id} is not readable after its write"))
    })();
    match work {
        Ok(note) => ok(json!({"row": note})),
        Err(e) => fail(1, e),
    }
}

/// `/note-list`: a task's notes, oldest first. `latest: n` keeps
/// only the newest n (still oldest first); removed notes appear
/// only with `include_removed: true`. `count` is always the task's
/// live (not removed) note total, so a bounded read says how many
/// it left out.
pub(crate) fn verb_note_list(t: &TursoBackend, req: &Value) -> Value {
    let Some(task_id) = req_i64(req, "task_id") else {
        return fail(2, "note-list: task_id is required");
    };
    let include_removed = req_bool(req, "include_removed").unwrap_or(false);
    let limit = req_i64(req, "latest")
        .or_else(|| req_i64(req, "limit"))
        .unwrap_or(1000)
        .clamp(1, 1000);
    let live = if include_removed {
        ""
    } else {
        " AND deleted_at IS NULL"
    };
    let work = (|| -> Result<Value, String> {
        let rows = t.query_objs(
            &format!(
                "SELECT {NOTE_COLS} FROM (SELECT {NOTE_COLS} FROM task_notes \
                 WHERE task_id = ?{live} ORDER BY id DESC LIMIT {limit}) ORDER BY id ASC"
            ),
            &[Arg::Int(task_id)],
            &[],
        )?;
        let count = t
            .exec(
                "SELECT count(*) FROM task_notes WHERE task_id = ? AND deleted_at IS NULL",
                &[Arg::Int(task_id)],
            )?
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        Ok(json!({"task_id": task_id, "rows": rows, "count": count}))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => fail(1, e),
    }
}

/// Why an author-only note write changed nothing.
fn refusal(t: &TursoBackend, verb: &str, note_id: i64, author: &str) -> String {
    match get_note(t, note_id) {
        Ok(None) => format!("{verb}: note {note_id} does not exist"),
        Ok(Some(n)) if !n["deleted_at"].is_null() => {
            format!("{verb}: note {note_id} was removed")
        }
        Ok(Some(n)) if n["author"].as_str() != Some(author) => format!(
            "{verb}: note {note_id} belongs to {}; only its author may change it",
            n["author"].as_str().unwrap_or("another author")
        ),
        Ok(Some(_)) => format!("{verb}: note {note_id} changed concurrently; retry"),
        Err(e) => e,
    }
}

/// `/note-edit`: replace a note's body (author-only). The prior body
/// lands in the `note_edited` history row's detail before the row
/// changes, inside the same batch.
pub(crate) fn verb_note_edit(t: &TursoBackend, req: &Value) -> Value {
    let Some(note_id) = req_i64(req, "note_id") else {
        return fail(2, "note-edit: note_id is required");
    };
    let author = match note_author(req) {
        Ok(a) => a,
        Err(e) => return fail(2, e),
    };
    let body = match validate_note_body(req) {
        Ok(b) => b,
        Err(e) => return fail(2, e),
    };
    note_write(t, "note-edit", note_id, &author, Some(&body))
}

/// `/note-remove`: soft-delete a note (author-only).
pub(crate) fn verb_note_remove(t: &TursoBackend, req: &Value) -> Value {
    let Some(note_id) = req_i64(req, "note_id") else {
        return fail(2, "note-remove: note_id is required");
    };
    let author = match note_author(req) {
        Ok(a) => a,
        Err(e) => return fail(2, e),
    };
    note_write(t, "note-remove", note_id, &author, None)
}

fn note_write(
    t: &TursoBackend,
    verb: &str,
    note_id: i64,
    author: &str,
    new_body: Option<&str>,
) -> Value {
    let now = now_iso();
    let (kind, change_sql, change_args) = match new_body {
        Some(b) => (
            "note_edited",
            "UPDATE task_notes SET body = ?, edited_at = ? \
             WHERE id = ? AND author = ? AND deleted_at IS NULL",
            vec![
                Arg::Text(b.to_string()),
                Arg::Text(now.clone()),
                Arg::Int(note_id),
                Arg::Text(author.to_string()),
            ],
        ),
        None => (
            "note_removed",
            "UPDATE task_notes SET deleted_at = ? \
             WHERE id = ? AND author = ? AND deleted_at IS NULL",
            vec![
                Arg::Text(now.clone()),
                Arg::Int(note_id),
                Arg::Text(author.to_string()),
            ],
        ),
    };
    // The event reads the row as it stands before the change, so
    // previous_body is the superseded (or removed) text. It selects
    // with the change's own predicate: a refused write inserts no
    // event, so HISTORY_INSERT_SQL's changes() = 1 writes no history.
    let event_sql = "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
        SELECT ?, task_id, ?, json_object('actor', ?, 'note_id', id, 'previous_body', body, \
            'session_id', session_id, 'model_provider', model_provider, \
            'model_id', model_id, 'model_variant', model_variant), ? \
        FROM task_notes WHERE id = ? AND author = ? AND deleted_at IS NULL";
    let work = (|| -> Result<Value, String> {
        let results = t.exec_batch(&[
            ("BEGIN IMMEDIATE", vec![]),
            (
                event_sql,
                vec![
                    Arg::Text(author.to_string()),
                    Arg::Text(kind.to_string()),
                    Arg::Text(author.to_string()),
                    Arg::Text(now.clone()),
                    Arg::Int(note_id),
                    Arg::Text(author.to_string()),
                ],
            ),
            (HISTORY_INSERT_SQL, vec![Arg::Null, Arg::Null]),
            (change_sql, change_args),
            ("COMMIT", vec![]),
        ])?;
        if results.get(3).map(|r| r.affected) != Some(1) {
            return Err(refusal(t, verb, note_id, author));
        }
        get_note(t, note_id)?
            .ok_or_else(|| format!("{verb}: note {note_id} is not readable after its write"))
    })();
    match work {
        Ok(note) => ok(json!({"row": note})),
        Err(e) => fail(1, e),
    }
}
