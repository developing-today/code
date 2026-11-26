//! Session-layer admin audit (#1063): one audit line per admin frame, in the
//! actor's record shape. Split out of `world_session/mod.rs` to keep that
//! file under the production-line limit (#13057).

use super::*;

/// Wall-clock milliseconds for session-layer audit records (the actor
/// stamps records with its world clock; hub and session frames are
/// processed before any world clock is involved).
fn audit_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// The bare audit action name of an admin frame, for dispatch sites
/// that refuse several admin frame kinds at one point.
pub(super) fn admin_frame_name(frame: &ClientFrame) -> &'static str {
    match frame {
        ClientFrame::Invite { .. } => "Invite",
        ClientFrame::Compile { .. } => "Compile",
        ClientFrame::CreateWorld { .. } => "CreateWorld",
        ClientFrame::ListWorlds { .. } => "ListWorlds",
        ClientFrame::InstallBegin { .. } => "InstallBegin",
        ClientFrame::EventHistoryAdmin { .. } => "EventHistoryAdmin",
        ClientFrame::SetExecutionProfile { .. } => "SetExecutionProfile",
        ClientFrame::SetPresentationPolicy { .. } => "SetPresentationPolicy",
        _ => "Unknown",
    }
}

/// Emit the one audit line an admin frame produces (#1063): the
/// session-layer half of the audit stream, in the actor's record shape
/// on the same reserved target (`id::audit`). The actor is the literal
/// `admin` — the token itself is a secret and is never recorded. Every
/// dispatch path through this module calls this exactly once per admin
/// frame (the [`frame_admin_token`] set), so every admin frame
/// produces exactly one line, denials included.
pub(super) fn audit_admin_frame(
    world_id: &str,
    action: &str,
    outcome: &str,
    parameters: serde_json::Value,
) {
    emit_audit_record(&AuditRecord {
        schema_version: 2,
        timestamp_ms: audit_now_ms(),
        world_id: world_id.to_owned(),
        actor: "admin".to_owned(),
        action: format!("frame:{action}"),
        parameters,
        outcome: outcome.to_owned(),
        target: world_id.to_owned(),
        correlation_id: next_audit_correlation_id(),
    });
}
