//! turso_queue::lifecycle_limits — lane assignment flips, platform
//! limitations, and session-to-session transfer guards (#16255).
//!
//! Split from turso_queue::lifecycle for #16255 (production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// Whether an EXISTING ledger row flips to direct (#8187): only an
/// opencode row, only when the caller passes --exclusion-reason
/// (a Muse lane taking a sink/legacy opencode task direct), and
/// never once a session is bound (session_id set) — a bound row's
/// assignment is history, not a routing decision. Claimant flows
/// that pass no exclusion keep their existing assignment.
pub(crate) fn lane_reassignment_flip(
    assigned_lane: &str,
    session_id: Option<&str>,
    exclusion: Option<&str>,
) -> bool {
    assigned_lane == "opencode"
        && exclusion.is_some()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Whether an EXISTING direct ledger row flips to opencode
/// (#8210): only a direct row, only when the caller passes
/// --opencode-reason (a Muse session sending its task to
/// opencode with a stated reason), and never once a session is
/// bound — the mirror of lane_reassignment_flip. Before #8210 the
/// existing-row path of verb_lane only flipped opencode -> direct,
/// so a lane that had claimed a task (fresh direct ledger row)
/// had no way to record an opencode assignment on it at all.
pub(crate) fn lane_flip_to_opencode(
    assigned_lane: &str,
    session_id: Option<&str>,
    opencode_reason: Option<&str>,
) -> bool {
    assigned_lane == "direct"
        && !opencode_reason.map(str::trim).unwrap_or("").is_empty()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Platform limitation validity (#8210 — Drew: a task may be
/// limited to muse or opencode "but better be good reason"). A
/// limitation is an EXCEPTION beside the open default: it exists
/// only with a reason recorded next to it. A stored limitation
/// whose reason is blank is invalid — never enforced; the
/// transfer path clears it and records the clearing.
pub(crate) fn platform_limit_invalid(limit: Option<&str>, reason: Option<&str>) -> bool {
    !limit.map(str::trim).unwrap_or("").is_empty() && reason.map(str::trim).unwrap_or("").is_empty()
}

/// Why a requested limitation cannot be recorded, or None when
/// it can: the family must be a real platform family (#8187's
/// vocabulary), and the reason must be present and non-blank —
/// the lane verb refuses the set otherwise. A reason without a
/// limitation is likewise refused (it would record nothing).
pub(crate) fn platform_limit_error(limit: Option<&str>, reason: Option<&str>) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    let r = reason.map(str::trim).unwrap_or("");
    if l.is_empty() {
        if !r.is_empty() {
            return Some("platform_limit_reason requires a platform_limit to limit".to_string());
        }
        return None;
    }
    if let Err(e) = parse_platform(Some(l)) {
        return Some(e);
    }
    if r.is_empty() {
        return Some(format!(
            "a platform limitation requires a recorded reason \
             (--limit-reason): a task is limited to '{l}' only for \
             a good reason, recorded on the task"
        ));
    }
    None
}

/// Transfer refusal for a platform limitation (#8210), or None
/// when the limitation does not bind this claim: no limitation,
/// an invalid (reason-less) limitation — those are cleared, not
/// enforced — or a limitation naming the claimer's own family.
/// A valid limitation refuses every other family, and refuses a
/// claim that declares no family at all (it cannot show it is
/// the allowed one). The refusal names the limitation and its
/// recorded reason.
pub(crate) fn platform_limit_refusal(
    limit: Option<&str>,
    reason: Option<&str>,
    family: Option<&str>,
) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    if l.is_empty() || platform_limit_invalid(limit, reason) {
        return None;
    }
    let r = reason.map(str::trim).unwrap_or("");
    match family {
        Some(f) if f == l => None,
        Some(f) => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             a '{f}' transfer-claim cannot take it"
        )),
        None => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             the transfer-claim must declare its platform family to take it"
        )),
    }
}

/// Session-to-session transfer refusal (#8210), or None when the
/// transfer may proceed. Pure over the gathered marks so the
/// tests pin every guard without a live store; claim_transfer
/// runs this BEFORE any mutation, so a refusal leaves the
/// original claim untouched. Guards, in order:
/// - the row must be claimed under a live lease — terminal rows,
///   queued rows and parked rows (genuine broker wait-parks stay
///   excluded from by-id taking, exactly as by_id_claimable has
///   them) are not transfer sources;
/// - from_session must match a holder session mark: the bound
///   progress.session_id, a progress.from_session the holder
///   checkpointed, or the session on the task's latest claimed
///   event — only the current holder can give a task away;
/// - a row already bound to a session that is neither the passer
///   nor the claiming session refuses: that binding is a live
///   launch, not a pass.
pub(crate) fn transfer_refusal(
    row: &Value,
    from_session: &str,
    claimed_event_session: Option<&str>,
    ledger_session: Option<&str>,
    new_session: Option<&str>,
    now: &str,
) -> Option<String> {
    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "claimed" {
        return Some(format!(
            "task is {status}, not claimed: only a live claim can be transferred"
        ));
    }
    let lease = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if lease <= now {
        return Some("the holder's lease is not live: no live claim to transfer".to_string());
    }
    if from_session.trim().is_empty() {
        return Some("from_session is empty".to_string());
    }
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let bound = progress
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let offered = progress
        .get("from_session")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let marks = [bound, offered, claimed_event_session.unwrap_or("")];
    if !marks.iter().any(|m| !m.is_empty() && *m == from_session) {
        return Some(
            "from_session does not match the current holder's session mark: \
             only the current holder can give this task away"
                .to_string(),
        );
    }
    for existing in [bound, ledger_session.unwrap_or("")] {
        if !existing.is_empty() && existing != from_session && Some(existing) != new_session {
            return Some(format!(
                "task is already bound to session {existing}: \
                 a bound launch is not a transfer"
            ));
        }
    }
    None
}

/// The transfer UPDATE (#8210): one conditional statement moves
/// a live claim holder -> claimer. Status stays 'claimed',
/// attempts are NOT touched (a transfer is not a new attempt),
/// and the WHERE re-checks the status and the HOLDER worker read
/// above, so a competing transition in between loses cleanly
/// (0 rows -> refusal, original claim intact).
pub(crate) fn transfer_update_sql() -> &'static str {
    "UPDATE tasks SET worker_id = ?, claimed_at = ?, lease_until = ?, \
       lane_id = ?, platform = COALESCE(?, platform), \
       park_kind = NULL, wait_kind = NULL, not_before = NULL, updated_at = ? \
     WHERE id = ? AND status = 'claimed' AND worker_id = ?"
}
