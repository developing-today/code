//! Seat-side typed helpers for the actions-exec broker verbs
//! (#13774): live short-lived exec sessions inside GitHub Actions
//! runners. The runner-side agent does NOT use this client — it
//! speaks to the broker's public door with the job's OIDC token;
//! these helpers are the seat's half, over the standard queue
//! client transport (broker token, laptop broker endpoint).

use serde_json::{Value, json};

use super::transport::call_result;

/// POST one actions-exec verb through the shared client transport.
/// The envelope is returned as-is ({ok:true, ...} or the broker's
/// {ok:false, exit, error}); transport failure is exit 3.
fn ae_call(path: &str, body: Value) -> Result<Value, (i32, String)> {
    call_result(path, &body)
}

/// Open a session row. `lease_s` is the caller's --timeout (required
/// by the verb; there is no default), `max_s` the hard lifespan cap.
pub fn ae_open(
    host_repo: &str,
    workflow: &str,
    seat: &str,
    lease_s: i64,
    max_s: i64,
    hide_logs: bool,
) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-open",
        json!({
            "host_repo": host_repo,
            "workflow": workflow,
            "seat": seat,
            "lease_s": lease_s,
            "max_s": max_s,
            "hide_logs": hide_logs,
        }),
    )
}

/// Bind the dispatched workflow run to the session (the handoff).
pub fn ae_bind(handle: &str, run_id: i64, run_attempt: i64) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-bind",
        json!({"handle": handle, "run_id": run_id, "run_attempt": run_attempt}),
    )
}

/// Enqueue one command on a bound/live session.
pub fn ae_command(handle: &str, text: &str, timeout_s: i64) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-command",
        json!({"handle": handle, "text": text, "timeout_s": timeout_s}),
    )
}

/// Read session state + command states + IO chunks after `after_id`.
pub fn ae_stream(
    handle: &str,
    after_id: i64,
    cmd_id: Option<&str>,
) -> Result<Value, (i32, String)> {
    let mut body = json!({"handle": handle, "after_id": after_id});
    if let Some(cmd) = cmd_id {
        body["cmd_id"] = json!(cmd);
    }
    ae_call("/actions-exec-stream", body)
}

/// Renew the lease (clamped at the max lifespan by the broker;
/// `extension_required` in the reply says the ask passed the max).
pub fn ae_renew(handle: &str, lease_s: i64) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-renew",
        json!({"handle": handle, "lease_s": lease_s}),
    )
}

/// Request an emergency extension past the max lifespan. The
/// broker records the request and raises the needs-drew alert.
pub fn ae_extend_request(
    handle: &str,
    extend_s: i64,
    reason: &str,
) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-extend-request",
        json!({"handle": handle, "extend_s": extend_s, "reason": reason}),
    )
}

/// Record the decision on a pending extension request.
pub fn ae_extend_decide(handle: &str, approve: bool) -> Result<Value, (i32, String)> {
    ae_call(
        "/actions-exec-extend-decide",
        json!({"handle": handle, "approve": approve}),
    )
}

/// Close a session (seat side; the agent side closes through the
/// OIDC door instead).
pub fn ae_close(handle: &str) -> Result<Value, (i32, String)> {
    ae_call("/actions-exec-close", json!({"handle": handle}))
}

/// The #13765 placement-coordinator claim attempt. The verbs are
/// not live yet: an unknown-command answer (or any error) comes
/// back as Err/ok:false and the caller proceeds on the singular
/// path, naming it. Returns Ok(true) only when a coordinator
/// actually granted the claim.
pub fn ae_placement_claim(host_repo: &str, seat: &str, handle: &str) -> bool {
    let body = json!({
        "repo": host_repo,
        "seat": seat,
        "session": handle,
        "kind": "actions-exec",
    });
    match call_result("/placement-claim", &body) {
        Ok(v) => {
            v.get("ok").and_then(|o| o.as_bool()) == Some(true)
                && v.get("granted").and_then(|g| g.as_bool()) != Some(false)
        }
        Err(_) => false,
    }
}
