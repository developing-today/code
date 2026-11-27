//! turso_queue::admin_time — time ready_condition recheck pass (#14212).
//!
//! Split from turso_queue::admin for #16255 (production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// Parse a stored ready_condition `until` to unix seconds
/// (#14212). Stored values arrive in both shapes the issue
/// names — `2026-11-16T00:14:04Z` and
/// `2026-11-03T16:03:00.171848+00:00` — so the parser accepts a
/// trailing `Z` or a numeric `±HH:MM` offset and an optional
/// fractional-seconds part. (iso_to_unix handles neither the
/// offset form nor, with an offset present, the fraction: its
/// seconds field is parsed as one f64.) Returns None for
/// anything else; an unparseable `until` is never due, so a
/// malformed condition surfaces instead of releasing early.
pub(crate) fn time_until_unix(raw: &str) -> Option<i64> {
    let (head, offset_secs) = if let Some(h) = raw.strip_suffix('Z') {
        (h, 0i64)
    } else if raw.len() > 6 {
        let (h, off) = raw.split_at(raw.len() - 6);
        let bytes = off.as_bytes();
        if (bytes[0] == b'+' || bytes[0] == b'-') && bytes[3] == b':' {
            let oh: i64 = off[1..3].parse().ok()?;
            let om: i64 = off[4..6].parse().ok()?;
            let sign = if bytes[0] == b'-' { -1 } else { 1 };
            (h, sign * (oh * 3600 + om * 60))
        } else {
            (raw, 0)
        }
    } else {
        (raw, 0)
    };
    let head = match head.split_once('.') {
        Some((h, frac)) if frac.bytes().all(|b| b.is_ascii_digit()) && !frac.is_empty() => h,
        Some(_) => return None,
        None => head,
    };
    iso_to_unix(head).map(|u| u - offset_secs)
}

/// The #14212 time recheck verdict for one row — the pure
/// decision the sweep driver and the /time-recheck verb share.
/// A row is this pass's business only when it is blocked, its
/// ready_condition is {"kind": "time", "until": …}, and that
/// time has passed. Two owners outrank the timer, exactly as
/// they outrank the quarantine rechecker: a dep record (the dep
/// rechecker owns the row, #8276) and a lane block (verb_block's
/// blocker/reason record, #10714) — a stale time condition left
/// on such a row must never undo the block that owns it today.
/// The six rows behind #14212 carry neither.
#[derive(Debug, PartialEq)]
pub(crate) enum TimeRecheck {
    NotApplicable,
    /// The recorded wait time has passed: requeue the row.
    Release,
    /// A time condition whose time has not come (or cannot be
    /// parsed): the block stands.
    NotDue,
}

pub(crate) fn time_recheck_verdict(row: &Value, now_unix_v: i64) -> TimeRecheck {
    if row.get("status").and_then(|v| v.as_str()) != Some("blocked") {
        return TimeRecheck::NotApplicable;
    }
    if dep_block_active(row) || lane_blocked(row) {
        return TimeRecheck::NotApplicable;
    }
    let condition = row.get("ready_condition");
    let kind = condition
        .and_then(|c| c.get("kind"))
        .and_then(|v| v.as_str());
    if kind != Some("time") {
        return TimeRecheck::NotApplicable;
    }
    let until = condition
        .and_then(|c| c.get("until"))
        .and_then(|v| v.as_str())
        .and_then(time_until_unix);
    match until {
        Some(u) if u <= now_unix_v => TimeRecheck::Release,
        _ => TimeRecheck::NotDue,
    }
}

/// Apply one row's time recheck verdict. Release requeues the
/// row and clears every wait field the condition owned
/// (ready_condition, not_before, park_kind, wait_kind), with an
/// `unblocked` event naming the lapsed condition — the same
/// shape verb_unblock produces, audited as the rechecker's own
/// decision. The conditional UPDATE keeps a row that changed
/// state mid-pass blocked.
pub(crate) fn time_recheck_one(
    t: &TursoBackend,
    row: &Value,
    now_unix_v: i64,
) -> Result<&'static str, TErr> {
    let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    match time_recheck_verdict(row, now_unix_v) {
        TimeRecheck::NotApplicable => Ok("not_applicable"),
        TimeRecheck::NotDue => Ok("not_due"),
        TimeRecheck::Release => {
            let condition = row.get("ready_condition").cloned().unwrap_or(Value::Null);
            let cleared = t.exec(
                "UPDATE tasks SET status = 'queued', not_before = NULL, \
                   park_kind = NULL, wait_kind = NULL, ready_condition = NULL, \
                   updated_at = ? \
                 WHERE id = ? AND status = 'blocked'",
                &[Arg::Text(now_iso()), Arg::Int(id)],
            )?;
            if cleared.affected == 0 {
                return Ok("release_raced");
            }
            event(
                t,
                Some("time-rechecker"),
                Some(id),
                "unblocked",
                &json!({
                    "reason": "blocked until a time that has passed; the wait lapsed (#14212)",
                    "via": "time_recheck",
                    "discarded_condition": condition,
                }),
            )?;
            Ok("released")
        }
    }
}

/// The #14212 time recheck pass: every blocked row carrying a
/// time ready_condition is examined once; a row whose time has
/// passed is requeued automatically. Before this pass nothing
/// released such rows: the claim pick takes only queued /
/// parked / handed_off rows, the dep rechecker watches
/// dep-blocked rows only, the quarantine rechecker only
/// claim-path quarantines, and revalidate_parked_tasks both
/// selects parked rows only and scores kind=time "confirmed"
/// forever — so the oldest of the six sat ready for three
/// weeks. Rides the sweep beside the dep and quarantine
/// recheckers (same minutely timer, same opt-out shape), and
/// /time-recheck runs it directly. Candidate selection is by
/// the stored condition's kind in SQL; the due test itself is
/// time_recheck_verdict's, so both stored timestamp shapes are
/// judged by parsed instant, never by text order.
pub fn run_time_rechecks(t: &TursoBackend, limit: i64) -> Result<Value, TErr> {
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status = 'blocked' \
             AND ready_condition IS NOT NULL \
             AND json_extract(ready_condition, '$.kind') = 'time' \
             ORDER BY updated_at ASC LIMIT {limit}"
        ),
        &[],
        &TASK_JSON_COLS,
    )?;
    let now_unix_v = now_unix();
    let (mut checked, mut released, mut not_due, mut raced) = (0i64, 0i64, 0i64, 0i64);
    for row in &rows {
        match time_recheck_one(t, row, now_unix_v)? {
            "released" => {
                checked += 1;
                released += 1;
            }
            "not_due" => {
                checked += 1;
                not_due += 1;
            }
            "release_raced" => raced += 1,
            _ => {}
        }
    }
    Ok(json!({
        "checked": checked, "released": released,
        "not_due": not_due, "raced": raced,
    }))
}

/// /time-recheck (#14212): run the recheck pass, or — with a
/// task_id — force one blocked task's recheck right now. The
/// manual trigger lanes and the monitor use to prove the
/// machinery (and to release the six fossil rows on deploy);
/// production cadence is the sweep's.
pub(crate) fn verb_time_recheck(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        if let Some(task_id) = req_i64(req, "task_id") {
            let Some(row) = get_task(t, task_id)? else {
                return Ok(json!({"task_id": task_id, "outcome": "no_such_task"}));
            };
            if row.get("status").and_then(|v| v.as_str()) != Some("blocked") {
                return Ok(json!({"task_id": task_id, "outcome": "not_blocked"}));
            }
            let outcome = time_recheck_one(t, &row, now_unix())?;
            let after = get_task(t, task_id)?.unwrap_or(Value::Null);
            return Ok(json!({
                "task_id": task_id, "outcome": outcome, "row": after,
            }));
        }
        run_time_rechecks(t, req_i64(req, "limit").unwrap_or(200))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}
