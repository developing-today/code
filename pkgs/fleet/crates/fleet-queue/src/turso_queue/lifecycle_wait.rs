//! turso_queue::lifecycle_wait — park wait-kind mapping and wait-floor
//! helpers for verb_park / verb_wake / verb_release (#16255).
//!
//! Split from turso_queue::lifecycle for #16255 (production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// wait_kind mapping, copied from the PG park_task body (0023 §3).
pub(crate) fn wait_kind(condition: &Value) -> &'static str {
    match condition.get("kind").and_then(|k| k.as_str()) {
        Some("time") => "timer",
        Some("triage") => "triage",
        Some("ci") | Some("ci_green") | Some("pr_merged") | Some("issue_closed")
        | Some("upstream_pr") => "ci",
        Some("review") | Some("codex_review") | Some("pr_review") | Some("person_answer") => {
            "review"
        }
        Some("capacity") | Some("quota_reset") => "capacity",
        _ => "external",
    }
}

/// Broker-owned minimum wait TTLs (#8184): whatever delay a lane
/// asks for, a recorded wait is held at least this long. Lanes
/// re-discovering an unchanged wait every 20-25 minutes was the
/// park churn: one discovery park per wait at these scales costs
/// ~3% of the cycles. Floors: triage waits 6h (the triage step
/// works on its own cadence and wakes the item via /wake when
/// done), ci and capacity waits 1h (backlog-scale, not
/// poll-scale). Timer/review/external waits keep the caller's
/// delay — those parks carry an explicit owner-chosen horizon.
pub(crate) fn wait_floor_seconds(wait_kind: &str) -> Option<i64> {
    match wait_kind {
        "triage" => Some(21_600),
        "ci" | "capacity" => Some(3_600),
        _ => None,
    }
}

/// Apply the wait-kind floor to a park's not_before: a requested
/// wake earlier than the floor is bumped to the floor (and the
/// park becomes relative); a later one stands.
pub(crate) fn apply_wait_floor(
    wait_kind: &str,
    not_before: String,
    park_kind: &'static str,
) -> (String, &'static str) {
    match wait_floor_seconds(wait_kind) {
        Some(floor) => {
            let earliest = after_iso(floor);
            if not_before < earliest {
                (earliest, "relative")
            } else {
                (not_before, park_kind)
            }
        }
        None => (not_before, park_kind),
    }
}

/// #9179 class (c)4: a park carrying a structured time condition
/// (`{"kind": "time", "until": <rfc3339>}`) must not lapse before
/// that condition's own horizon. The audit's live case (task
/// 13224) was parked for a day while its recorded resume
/// condition was a month out, so it lapsed into the unchanged
/// condition and burned a lane pass re-parking. When the
/// condition's `until` parses and is later than the requested
/// lapse term, the lapse term rises to `until` (and the park
/// becomes absolute — its term is now that fixed timestamp).
/// An earlier, equal, missing, or unparseable `until`, and any
/// non-time condition, leave the park exactly as requested.
pub(crate) fn apply_time_condition_floor(
    condition: &Value,
    not_before: String,
    park_kind: &'static str,
) -> (String, &'static str) {
    if condition.get("kind").and_then(|k| k.as_str()) != Some("time") {
        return (not_before, park_kind);
    }
    let Some(until) = condition.get("until").and_then(|u| u.as_str()) else {
        return (not_before, park_kind);
    };
    let (Some(until_epoch), Some(nb_epoch)) = (
        crate::disposition::parse_rfc3339_epoch(until),
        crate::disposition::parse_rfc3339_epoch(&not_before),
    ) else {
        return (not_before, park_kind);
    };
    if until_epoch > nb_epoch {
        (until.to_string(), "absolute")
    } else {
        (not_before, park_kind)
    }
}

/// #3418: the required budget a park records. An explicit
/// `required_budget_minutes` in the park request wins. Otherwise a
/// capacity wait (ready-condition kind `capacity`) defaults the
/// requirement from the packet's own `timebox_minutes` — the
/// natural default source in the #3418 design sketch — so a
/// capacity park is budget-matched without the parker restating
/// the packet's number. Any other park records no requirement and
/// keeps behaving exactly as before. Non-positive values record
/// no requirement.
pub(crate) fn park_required_budget(
    explicit: Option<i64>,
    condition: &Value,
    payload: &Value,
) -> Option<i64> {
    if let Some(v) = explicit {
        return (v > 0).then_some(v);
    }
    if condition.get("kind").and_then(|k| k.as_str()) == Some("capacity") {
        return payload
            .get("timebox_minutes")
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0);
    }
    None
}

/// #9178 §2 (FDR-0002): the live manual-claim override a park must
/// honor. Returns the recorded audit when the row carries a
/// `result.manual_override` bound to the CURRENT claim and the
/// incoming park's (wait_kind, ready_condition) equals the recorded
/// previous pair — the owner already overrode exactly this
/// condition, so a boot-time re-park for it must refuse. Anything
/// else (no audit, a materially new condition, or a stale marker
/// from an earlier claim — every claim stamps a fresh claimed_at,
/// so the audit's claimed_at only matches the live claim) parks as
/// before.
pub(crate) fn manual_override_park_cover(
    row: &Value,
    wait_kind: &str,
    condition: &Value,
) -> Option<Value> {
    let audit = row.get("result")?.get("manual_override")?;
    if !audit.is_object() {
        return None;
    }
    if audit.get("claimed_at") != row.get("claimed_at") {
        return None;
    }
    if audit.get("previous_wait_kind").and_then(Value::as_str) != Some(wait_kind) {
        return None;
    }
    let previous = audit
        .get("previous_ready_condition")
        .unwrap_or(&Value::Null);
    if !same_wait_condition(previous, condition, wait_kind) {
        return None;
    }
    Some(audit.clone())
}

/// #16392: is the incoming park's condition the SAME wait the audit
/// recorded, for cover purposes? Exact equality is the base case,
/// but a time condition's `until` is a lapse horizon minted fresh
/// on every bare park (now + delay) — it is not what makes the wait
/// the wait (#9179 (c)4's own framing). Two time conditions are
/// therefore the same wait when everything except `until` matches,
/// and a recorded NULL condition (older rows parked before a
/// condition was always stored) matches a bare timer re-park: the
/// recorded wait carried no explicit condition, so there is no
/// materially new condition to honor instead.
fn same_wait_condition(previous: &Value, incoming: &Value, wait_kind: &str) -> bool {
    if previous == incoming {
        return true;
    }
    if previous.is_null() {
        return wait_kind == "timer"
            && incoming.get("kind").and_then(Value::as_str) == Some("time");
    }
    let strip_until = |v: &Value| -> Value {
        let mut obj = match v.as_object() {
            Some(o) => o.clone(),
            None => return v.clone(),
        };
        obj.remove("until");
        Value::Object(obj)
    };
    previous.get("kind").and_then(Value::as_str) == Some("time")
        && incoming.get("kind").and_then(Value::as_str) == Some("time")
        && strip_until(previous) == strip_until(incoming)
}
