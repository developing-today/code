//! turso_queue::actions_exec — live, short-lived exec sessions
//! inside GitHub Actions runners (developing-today/code#13774).
//!
//! The broker half of `fleet actions exec`. A seat opens a session,
//! dispatches the fleet-exec workflow with the session handle, and
//! binds the resulting run; the runner-side agent then proves it is
//! inside that run with the job's GitHub OIDC token and registers.
//! Commands flow seat -> broker -> agent over these verbs; IO flows
//! back the same way. Nothing connection-shaped exists outside the
//! two endpoints: the handle is a public identifier, and the OIDC
//! token is presented only to the broker, in the X-Agent-Token
//! header (queue_broker's gate passes JWT-shaped tokens on the
//! agent paths through to these verbs, which verify them).
//!
//! Agent auth: the register/poll/io/result/close verbs carry the
//! job's OIDC JWT in the request's `_oidc_jwt` field (injected by
//! queue_broker from the X-Agent-Token header — never accepted from
//! the request body itself). Verification is RS256 against GitHub's
//! JWKS (cached), then claims against the session row the seat
//! created: issuer, expiry, `repository`, `job_workflow_ref`, and
//! the run binding (`run_id` / `run_attempt` — taken from the
//! verified claims at register time, then required to match).
//! The `aud` claim is deliberately not pinned in v1: GitHub's
//! default audience varies by instance, and the repository +
//! workflow + run binding is the substantive check.
//!
//! Lease discipline (the design's contract): the lease is set at
//! open; renewals may move it up to `max_until` and no further;
//! past the max only an extension moves it, and extensions are
//! tiered (design amendment on #13774): the first extension
//! within the auto caps is granted by /actions-exec-extend-request
//! itself with a notification-only push (tier 1); the second
//! pends under a deny window and is granted by silence at window
//! end unless Drew denies it first (tier 2); anything further —
//! a third extension, an oversized ask, or a request past the
//! cumulative auto cap — pends for Drew's manual answer (tier 3).
//! queue_broker raises the tier's PM push on the verb's success.
//! While a request is pending the session sits in
//! `extension-pending` with a decision grace past its lease (a
//! tier-2 request's hard deadline covers its whole deny window),
//! so a live agent is not killed mid-question; past the grace
//! (or the max, unextended) the session is expired and the
//! agent dies — the agent also enforces its own local deadline.
//!
//! Atomicity follows the house shape: state transitions are single
//! conditional UPDATEs whose WHERE clause re-checks the expected
//! prior state, so racing verbs resolve to exactly one winner.

use super::*;

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// DDL (referenced from core::SCHEMA, beside placement_sessions)
// ---------------------------------------------------------------------------

pub(crate) const ACTIONS_EXEC_DDL_SESSIONS: &str = "CREATE TABLE IF NOT EXISTS actions_exec_sessions (\
    handle TEXT PRIMARY KEY, host_repo TEXT NOT NULL, workflow TEXT NOT NULL, \
    seat TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'open', \
    hide_logs INTEGER NOT NULL DEFAULT 0, \
    run_id INTEGER, run_attempt INTEGER, \
    lease_until TEXT NOT NULL, max_until TEXT NOT NULL, \
    ext_state TEXT, ext_seconds INTEGER, \
    ext_count INTEGER NOT NULL DEFAULT 0, ext_total_s INTEGER NOT NULL DEFAULT 0, \
    ext_tier INTEGER, ext_decide_after TEXT, ext_reason TEXT, context TEXT, \
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, closed_at TEXT)";

pub(crate) const ACTIONS_EXEC_DDL_COMMANDS: &str = "CREATE TABLE IF NOT EXISTS actions_exec_commands (\
    cmd_id TEXT PRIMARY KEY, handle TEXT NOT NULL, seq INTEGER NOT NULL, \
    text TEXT NOT NULL, timeout_s INTEGER NOT NULL DEFAULT 60, \
    state TEXT NOT NULL DEFAULT 'queued', exit_code INTEGER, \
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL)";

pub(crate) const ACTIONS_EXEC_DDL_COMMANDS_IDX: &str = "CREATE INDEX IF NOT EXISTS actions_exec_commands_handle \
        ON actions_exec_commands(handle, seq)";

pub(crate) const ACTIONS_EXEC_DDL_IO: &str = "CREATE TABLE IF NOT EXISTS actions_exec_io (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, handle TEXT NOT NULL, \
    cmd_id TEXT NOT NULL, seq INTEGER NOT NULL, stream TEXT NOT NULL, \
    data TEXT NOT NULL, created_at TEXT NOT NULL)";

pub(crate) const ACTIONS_EXEC_DDL_IO_IDX: &str = "CREATE INDEX IF NOT EXISTS actions_exec_io_cmd \
        ON actions_exec_io(cmd_id, id)";

// ---------------------------------------------------------------------------
// Constants and small helpers
// ---------------------------------------------------------------------------

/// The workflow file the session agent runs under. The register
/// verb checks the OIDC `job_workflow_ref` against
/// `{host_repo}/.github/workflows/{workflow}@...`.
pub(crate) const DEFAULT_WORKFLOW: &str = "fleet-exec.yml";

/// Minimum lease the open verb accepts (the Action validates the
/// same floor runner-side).
const MIN_LEASE_S: i64 = 30;
/// Hard cap on max lifespan: the fleet-run leg cap (2 h).
const MAX_LIFESPAN_S: i64 = 7_200;
/// Default max lifespan when the seat does not name one (30 min).
const DEFAULT_MAX_S: i64 = 1_800;
/// One approved emergency extension adds at most this much.
const MAX_EXTENSION_S: i64 = 1_800;
/// While an extension request awaits Drew's answer, the session
/// may outlive its lease by this much before it expires.
const EXTENSION_GRACE_S: i64 = 300;
/// Tiered extension policy (design amendment on #13774): the
/// largest single ask the auto tiers (1-2) may grant. Equal to
/// MAX_EXTENSION_S today, so the binding form of the rule is the
/// raw-ask check in evaluate_extension_tier: an ask for more
/// than this is a manual (tier 3) request even though the grant
/// itself would be clamped to the same bound.
const EXT_AUTO_SIZE_CAP_S: i64 = 1_800;
/// Extension number one is tier 1 (auto + notify).
const EXT_TIER1_MAX_COUNT: i64 = 1;
/// Extension number two is tier 2 (notify + deny window);
/// number three and beyond are tier 3 (manual only).
const EXT_TIER2_MAX_COUNT: i64 = 2;
/// Cumulative granted extension seconds the auto tiers may
/// account for over a session's life; a request that would push
/// the total past this is tier 3. (With the current size/count
/// caps the count cap binds first; this is an independent guard
/// so retuning one cap cannot silently widen the auto surface.)
const EXT_AUTO_TOTAL_CAP_S: i64 = 3_600;
/// Tier 2's deny window: the push states the extension WILL be
/// approved at window end unless Drew denies it first.
const EXT_DENY_WINDOW_S: i64 = 600;
/// Long-poll ceiling for /actions-exec-poll.
const MAX_POLL_WAIT_S: i64 = 25;
/// IO chunk cap per append (the broker caps bodies anyway; a chunk
/// larger than this is a client bug, not data).
const MAX_IO_CHUNK: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Signed decide envelope — S1 of #14043 (broker half).
//
// The aggregator-side applier carries Drew's recorded answer to the
// broker's decide verb. Seat calls stay exactly as they were (no
// `sig` field). A call carrying `sig` must also carry `answer_id`
// and `answered_at`, and `sig` must be the lowercase hex of
// HMAC-SHA256 over
//
//     handle|approve|answer_id|answered_at
//
// where `approve` is the literal string "true" or "false" matching
// the request's `approve` boolean, and `answered_at` is the house
// ISO stamp (`YYYY-MM-DDTHH:MM:SSZ`, the broker's `iso()` shape —
// a valid RFC 3339 profile). The HMAC key is the secret named by
// `APPLY_KEY_SECRET_NAME_DEFAULT` (overridable for rotation via
// `FLEET_ACTIONS_EXEC_APPLY_KEY_NAME`), resolved by name through
// fleet-secrets — the value never appears in the repo, logs, or
// the issue record. `answered_at` must fall inside
// `SIGNED_DECIDE_WINDOW_S` back from now (plus a small future
// skew for clock drift); anything older is refused as a replay.
// The window bounds replay; exactly-once across retries is the
// applier ledger's job (S3), keyed by `answer_id`. The signed form
// is accepted only on /actions-exec-extend-decide.
// ---------------------------------------------------------------------------

/// fleet-secrets name of the applier HMAC key (design §2; the
/// example name there is kept as the default — a slice that settles
/// a different name records it, never a value, on #14043).
pub(crate) const APPLY_KEY_SECRET_NAME_DEFAULT: &str = "pm-aggregator.actions-exec-apply-key";
/// Replay window for the signed envelope: an `answered_at` older
/// than this is refused.
pub(crate) const SIGNED_DECIDE_WINDOW_S: i64 = 600;
/// Clock-skew allowance into the future for `answered_at`.
pub(crate) const SIGNED_DECIDE_FUTURE_SKEW_S: i64 = 60;

/// Canonical signed message (design §2 envelope): the exact bytes
/// the aggregator signs and the broker verifies. `approve` renders
/// as "true"/"false" to match the JSON boolean. The S3 signer must
/// produce byte-identical input; `signed_decide_message_canonical`
/// pins it.
pub(crate) fn signed_decide_message(
    handle: &str,
    approve: bool,
    answer_id: &str,
    answered_at: &str,
) -> String {
    format!(
        "{}|{}|{}|{}",
        handle,
        if approve { "true" } else { "false" },
        answer_id,
        answered_at
    )
}

fn apply_key_secret_name() -> String {
    std::env::var("FLEET_ACTIONS_EXEC_APPLY_KEY_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| APPLY_KEY_SECRET_NAME_DEFAULT.to_string())
}

fn resolve_apply_key() -> Result<Vec<u8>, String> {
    let name = apply_key_secret_name();
    let config = fleet_config::Config::load().unwrap_or_default();
    fleet_secrets::Resolver::default_chain(&config)
        .resolve(&name)
        .map(|v| v.expose().as_bytes().to_vec())
        .map_err(|e| format!("applier apply key {name}: {e}"))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((val(b[i])? << 4) | val(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

/// Verify the signed envelope on a decide request carrying `sig`.
/// Pure over the key and the caller-supplied `now` so tests inject
/// both and no test touches the secrets chain. Returns the fail
/// Value to answer with on any refusal.
fn check_signed_envelope(req: &Value, key: &[u8], now: i64) -> Result<(), Value> {
    let Some(sig_hex) = req.get("sig").and_then(|v| v.as_str()) else {
        return Err(fail(2, "sig is required for a signed decide".to_string()));
    };
    let Some(answer_id) = req_str(req, "answer_id").filter(|s| !s.is_empty()) else {
        return Err(fail(2, "answer_id is required with sig".to_string()));
    };
    let Some(answered_at) = req_str(req, "answered_at").filter(|s| !s.is_empty()) else {
        return Err(fail(2, "answered_at is required with sig".to_string()));
    };
    let Some(stamp_unix) = parse_iso(answered_at) else {
        return Err(fail(
            2,
            "answered_at must be an ISO stamp (YYYY-MM-DDTHH:MM:SSZ)".to_string(),
        ));
    };
    if stamp_unix < now - SIGNED_DECIDE_WINDOW_S || stamp_unix > now + SIGNED_DECIDE_FUTURE_SKEW_S
    {
        return Err(fail(
            4,
            "signed decide envelope is outside the acceptance window (replay limit)".to_string(),
        ));
    }
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return Err(fail(2, "handle is required".to_string()));
    };
    let approve = req.get("approve").and_then(|v| v.as_bool()) == Some(true);
    let msg = signed_decide_message(handle, approve, answer_id, answered_at);
    let Some(sig) = hex_decode(sig_hex) else {
        return Err(fail(4, "sig is not hex HMAC-SHA256".to_string()));
    };
    let vkey = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::verify(&vkey, msg.as_bytes(), &sig)
        .map_err(|_| fail(4, "signed decide envelope signature does not verify".to_string()))
}

static RAND_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Random-enough identifier material for handles and command ids:
/// SHA-256 over the clock, the process id, a process counter and a
/// caller-supplied distinguisher. Handles are public identifiers,
/// not credentials — uniqueness, not secrecy, is the requirement.
fn rand_hex(distinguisher: &str, bytes: usize) -> String {
    use sha2::{Digest, Sha256};
    let n = RAND_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(now.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(n.to_le_bytes());
    h.update(distinguisher.as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex.chars().take(bytes * 2).collect()
}

/// Parse the house ISO stamp (`YYYY-MM-DDTHH:MM:SSZ`, as produced by
/// core::iso) back to unix seconds. Returns None on any other shape.
fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> { s[from..from + len].parse::<i64>().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hh, mi, ss) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    // days_from_civil (Howard Hinnant's algorithm, the inverse of
    // core::civil_from_days).
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mi * 60 + ss)
}

// ---------------------------------------------------------------------------
// Session rows
// ---------------------------------------------------------------------------

fn session_row(t: &TursoBackend, handle: &str) -> Result<Option<Value>, TErr> {
    // query_objs' third argument is the JSON-parse column list;
    // this schema carries no JSON columns, so it stays empty.
    let rows = t.query_objs(
        "SELECT handle, host_repo, workflow, seat, state, hide_logs, \
                run_id, run_attempt, lease_until, max_until, \
                ext_state, ext_seconds, ext_count, ext_total_s, ext_tier, \
                ext_decide_after, ext_reason, context, created_at, updated_at \
         FROM actions_exec_sessions WHERE handle = ?",
        &[Arg::Text(handle.to_string())],
        &[],
    )?;
    Ok(rows.into_iter().next())
}

fn session_json(row: &Value) -> Value {
    let lease_unix = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let max_unix = row
        .get("max_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    json!({
        "handle": row.get("handle").cloned().unwrap_or(Value::Null),
        "host_repo": row.get("host_repo").cloned().unwrap_or(Value::Null),
        "workflow": row.get("workflow").cloned().unwrap_or(Value::Null),
        "seat": row.get("seat").cloned().unwrap_or(Value::Null),
        "state": row.get("state").cloned().unwrap_or(Value::Null),
        "hide_logs": row.get("hide_logs").and_then(|v| v.as_i64()).unwrap_or(0) != 0,
        "run_id": row.get("run_id").cloned().unwrap_or(Value::Null),
        "run_attempt": row.get("run_attempt").cloned().unwrap_or(Value::Null),
        "lease_until": row.get("lease_until").cloned().unwrap_or(Value::Null),
        "lease_unix": lease_unix,
        "max_until": row.get("max_until").cloned().unwrap_or(Value::Null),
        "max_unix": max_unix,
        "ext_state": row.get("ext_state").cloned().unwrap_or(Value::Null),
        "ext_seconds": row.get("ext_seconds").cloned().unwrap_or(Value::Null),
        "ext_tier": row.get("ext_tier").cloned().unwrap_or(Value::Null),
        "ext_count": row.get("ext_count").and_then(|v| v.as_i64()).unwrap_or(0),
        "ext_total_s": row.get("ext_total_s").and_then(|v| v.as_i64()).unwrap_or(0),
        "ext_decide_after": row.get("ext_decide_after").cloned().unwrap_or(Value::Null),
        "ext_reason": row.get("ext_reason").cloned().unwrap_or(Value::Null),
        "context": row.get("context").cloned().unwrap_or(Value::Null),
    })
}

/// The last moment the agent may live, in unix seconds: the lease,
/// or — while an extension decision is pending — the lease plus
/// the decision grace. A pending tier-2 request instead runs to
/// the end of its deny window plus the grace: the session must
/// not expire mid-window, before its silence-approval can land.
/// Never past max_until unless an approved extension already
/// moved max_until (approvals update both).
fn hard_deadline_unix(row: &Value) -> i64 {
    let lease = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    if row.get("ext_state").and_then(|v| v.as_str()) != Some("pending") {
        return lease;
    }
    if row.get("ext_tier").and_then(|v| v.as_i64()) == Some(2) {
        let decide_after = row
            .get("ext_decide_after")
            .and_then(|v| v.as_str())
            .and_then(parse_iso)
            .unwrap_or(lease);
        return decide_after.max(lease) + EXTENSION_GRACE_S;
    }
    lease + EXTENSION_GRACE_S
}

/// The state the rest of the world should treat the session as
/// being in: a live-shaped row past its hard deadline reads as
/// expired (the transition is also written back lazily by poll).
fn effective_state(row: &Value, now: i64) -> String {
    let state = row
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if matches!(
        state.as_str(),
        "open" | "bound" | "live" | "extension-pending"
    ) && now > hard_deadline_unix(row)
    {
        return "expired".to_string();
    }
    state
}

fn require_session(t: &TursoBackend, req: &Value) -> Result<Value, Value> {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return Err(fail(2, "handle is required".to_string()));
    };
    match session_row(t, handle) {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(fail(5, format!("unknown session handle {handle}"))),
        Err(e) => Err(terr(e)),
    }
}

/// require_session + the tier-2 settle: a pending tier-2 request
/// whose deny window has lapsed is granted (silence approves)
/// before the caller reads any state off the row, and the fresh
/// row is returned. Verbs that read or decide session state use
/// this; the agent's poll settles inline in its loop instead.
fn require_session_settled(t: &TursoBackend, req: &Value) -> Result<Value, Value> {
    let row = require_session(t, req)?;
    let now = now_unix();
    match settle_extension(t, &row, now) {
        Ok(true) => {
            let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
            match session_row(t, handle) {
                Ok(Some(fresh)) => Ok(fresh),
                _ => Ok(row),
            }
        }
        Ok(false) => Ok(row),
        Err(e) => Err(terr(e)),
    }
}

// ---------------------------------------------------------------------------
// Extension tiers (#13774 design amendment)
// ---------------------------------------------------------------------------

/// Approved extensions recorded on the row so far.
fn ext_count(row: &Value) -> i64 {
    row.get("ext_count").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// Total granted extension seconds recorded on the row so far.
fn ext_total_s(row: &Value) -> i64 {
    row.get("ext_total_s").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// The tier ladder, evaluated at request time from the session's
/// counters and the ask. `raw_extend_s` is the ask as requested,
/// `extend_s` the same ask clamped to the per-approval bound:
/// an oversized ask is manual (tier 3) even though its grant
/// would be clamped. Pure — the tier tests name this as oracle.
pub(crate) fn evaluate_extension_tier(
    ext_count: i64,
    ext_total_s: i64,
    raw_extend_s: i64,
    extend_s: i64,
) -> i64 {
    if raw_extend_s > EXT_AUTO_SIZE_CAP_S || extend_s > EXT_AUTO_SIZE_CAP_S {
        return 3;
    }
    if ext_count >= EXT_TIER2_MAX_COUNT {
        return 3;
    }
    if ext_total_s + extend_s > EXT_AUTO_TOTAL_CAP_S {
        return 3;
    }
    if ext_count < EXT_TIER1_MAX_COUNT {
        1
    } else {
        2
    }
}

/// One extension grant, applied identically by every approval
/// path (tier-1 auto, tier-2 silence, manual decide): the max
/// and the lease move by the granted seconds, the counters
/// increment, and the outcome is recorded in ext_state.
struct ExtensionGrant {
    extend_s: i64,
    tier: i64,
    reason: String,
    outcome: &'static str,
}

/// Apply a grant as one conditional UPDATE. `from_pending`
/// selects the guard: a pending request must still be pending
/// (decide / settle paths); a fresh tier-1 grant must find no
/// request pending and the session in a live shape (request
/// path). Returns the new (max_until, lease_until) when the
/// UPDATE landed, None when the guard refused it.
fn apply_extension(
    t: &TursoBackend,
    row: &Value,
    now: i64,
    grant: &ExtensionGrant,
    from_pending: bool,
) -> Result<Option<(String, String)>, TErr> {
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let max_unix = row
        .get("max_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let lease_unix = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let new_max = iso(max_unix + grant.extend_s);
    let new_lease = iso(lease_unix
        .max(now + grant.extend_s)
        .min(max_unix + grant.extend_s));
    let guard = if from_pending {
        "ext_state = 'pending'"
    } else {
        "state IN ('open', 'bound', 'live', 'extension-pending') AND COALESCE(ext_state, '') <> 'pending'"
    };
    let sql = format!(
        "UPDATE actions_exec_sessions \
         SET state = 'live', ext_state = ?, ext_seconds = ?, ext_tier = ?, \
             ext_reason = ?, ext_decide_after = NULL, \
             ext_count = COALESCE(ext_count, 0) + 1, \
             ext_total_s = COALESCE(ext_total_s, 0) + ?, \
             max_until = ?, lease_until = ?, updated_at = ? \
         WHERE handle = ? AND {guard}"
    );
    let res = t.exec(
        &sql,
        &[
            Arg::Text(grant.outcome.to_string()),
            Arg::Int(grant.extend_s),
            Arg::Int(grant.tier),
            Arg::Text(grant.reason.clone()),
            Arg::Int(grant.extend_s),
            Arg::Text(new_max.clone()),
            Arg::Text(new_lease.clone()),
            Arg::Text(now_iso()),
            Arg::Text(handle.to_string()),
        ],
    )?;
    Ok(if res.affected > 0 {
        Some((new_max, new_lease))
    } else {
        None
    })
}

/// Settle a lapsed tier-2 deny window: silence approves. Fires
/// at most once — the grant's conditional UPDATE consumes the
/// pending state — and never for tier 3, which waits for Drew's
/// recorded answer however old the request gets.
fn settle_extension(t: &TursoBackend, row: &Value, now: i64) -> Result<bool, TErr> {
    if row.get("ext_state").and_then(|v| v.as_str()) != Some("pending") {
        return Ok(false);
    }
    if row.get("ext_tier").and_then(|v| v.as_i64()) != Some(2) {
        return Ok(false);
    }
    let Some(decide_after) = row
        .get("ext_decide_after")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
    else {
        return Ok(false);
    };
    if now < decide_after {
        return Ok(false);
    }
    let grant = ExtensionGrant {
        extend_s: row
            .get("ext_seconds")
            .and_then(|v| v.as_i64())
            .unwrap_or(MAX_EXTENSION_S)
            .clamp(60, MAX_EXTENSION_S),
        tier: 2,
        reason: row
            .get("ext_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        outcome: "silence-approved",
    };
    Ok(apply_extension(t, row, now, &grant, true)?.is_some())
}

// ---------------------------------------------------------------------------
// GitHub OIDC verification (agent auth)
// ---------------------------------------------------------------------------

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4 + 3);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}

struct JwksKey {
    kid: String,
    /// DER-encoded RSAPublicKey (SEQUENCE of modulus, exponent) —
    /// the shape ring's RSA verification takes.
    der: Vec<u8>,
}

struct JwksCache {
    fetched_unix: i64,
    keys: Vec<JwksKey>,
}

static JWKS: Mutex<Option<JwksCache>> = Mutex::new(None);
const JWKS_TTL_S: i64 = 600;

fn jwks_url() -> String {
    std::env::var("FLEET_ACTIONS_EXEC_JWKS_URL").unwrap_or_else(|_| {
        "https://token.actions.githubusercontent.com/.well-known/jwks".to_string()
    })
}

fn der_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.push(0x81);
        out.push(len as u8);
    } else {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push(len as u8);
    }
}

fn der_integer(out: &mut Vec<u8>, raw: &[u8]) {
    // Strip leading zero bytes, then re-add one if the high bit is
    // set (DER integers are signed).
    let mut body: &[u8] = raw;
    while body.len() > 1 && body[0] == 0 {
        body = &body[1..];
    }
    let pad = body.first().map(|b| b & 0x80 != 0).unwrap_or(false);
    out.push(0x02);
    der_len(out, body.len() + usize::from(pad));
    if pad {
        out.push(0);
    }
    out.extend_from_slice(body);
}

fn rsa_public_key_der(n_b64: &str, e_b64: &str) -> Option<Vec<u8>> {
    let n = b64url_decode(n_b64)?;
    let e = b64url_decode(e_b64)?;
    let mut content = Vec::new();
    der_integer(&mut content, &n);
    der_integer(&mut content, &e);
    let mut out = vec![0x30];
    der_len(&mut out, content.len());
    out.extend_from_slice(&content);
    Some(out)
}

fn jwks_keys(force: bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    let now = now_unix();
    if !force {
        if let Ok(guard) = JWKS.lock() {
            if let Some(cache) = guard.as_ref() {
                if now - cache.fetched_unix < JWKS_TTL_S {
                    return Ok(cache
                        .keys
                        .iter()
                        .map(|k| (k.kid.clone(), k.der.clone()))
                        .collect());
                }
            }
        }
    }
    let url = jwks_url();
    let mut resp = ureq::agent()
        .get(&url)
        .call()
        .map_err(|e| format!("jwks fetch {url}: {e}"))?;
    let doc: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("jwks parse: {e}"))?;
    let mut keys = Vec::new();
    if let Some(arr) = doc.get("keys").and_then(|k| k.as_array()) {
        for k in arr {
            if k.get("kty").and_then(|v| v.as_str()) != Some("RSA") {
                continue;
            }
            let (Some(kid), Some(n), Some(e)) = (
                k.get("kid").and_then(|v| v.as_str()),
                k.get("n").and_then(|v| v.as_str()),
                k.get("e").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            if let Some(der) = rsa_public_key_der(n, e) {
                keys.push(JwksKey {
                    kid: kid.to_string(),
                    der,
                });
            }
        }
    }
    if keys.is_empty() {
        return Err(format!("jwks at {url} carried no usable RSA keys"));
    }
    if let Ok(mut guard) = JWKS.lock() {
        *guard = Some(JwksCache {
            fetched_unix: now,
            keys: keys
                .iter()
                .map(|k| JwksKey {
                    kid: k.kid.clone(),
                    der: k.der.clone(),
                })
                .collect(),
        });
    }
    Ok(keys.into_iter().map(|k| (k.kid, k.der)).collect())
}

/// Verified OIDC claims, reduced to what the session binding needs.
struct OidcClaims {
    repository: String,
    job_workflow_ref: String,
    run_id: i64,
    run_attempt: i64,
}

fn verify_oidc(jwt: &str) -> Result<OidcClaims, String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err("token is not a three-segment JWT".to_string());
    }
    let header: Value =
        serde_json::from_slice(&b64url_decode(parts[0]).ok_or("JWT header is not base64url")?)
            .map_err(|e| format!("JWT header parse: {e}"))?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("RS256") {
        return Err("JWT alg is not RS256".to_string());
    }
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or("JWT header carries no kid")?;
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let signature = b64url_decode(parts[2]).ok_or("JWT signature is not base64url")?;
    let verify_with = |keys: &[(String, Vec<u8>)]| -> Result<(), String> {
        let der = keys
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, d)| d.clone())
            .ok_or_else(|| format!("no JWKS key for kid {kid}"))?;
        let public_key = ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            der,
        );
        public_key
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|_| "JWT signature does not verify".to_string())
    };
    if let Err(e) = verify_with(&jwks_keys(false)?) {
        // One refetch on failure: the kid may belong to a key GitHub
        // rotated in since the cache filled.
        verify_with(&jwks_keys(true)?).map_err(|_| e)?;
    }
    let claims: Value =
        serde_json::from_slice(&b64url_decode(parts[1]).ok_or("JWT payload is not base64url")?)
            .map_err(|e| format!("JWT payload parse: {e}"))?;
    if claims.get("iss").and_then(|v| v.as_str())
        != Some("https://token.actions.githubusercontent.com")
    {
        return Err("JWT issuer is not GitHub Actions".to_string());
    }
    let exp = claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    if exp <= now_unix() {
        return Err("JWT is expired".to_string());
    }
    let repository = claims
        .get("repository")
        .and_then(|v| v.as_str())
        .ok_or("JWT carries no repository claim")?
        .to_string();
    let job_workflow_ref = claims
        .get("job_workflow_ref")
        .and_then(|v| v.as_str())
        .ok_or("JWT carries no job_workflow_ref claim")?
        .to_string();
    let run_id = claims
        .get("run_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or("JWT run_id claim is not an integer string")?;
    let run_attempt = claims
        .get("run_attempt")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or("JWT run_attempt claim is not an integer string")?;
    Ok(OidcClaims {
        repository,
        job_workflow_ref,
        run_id,
        run_attempt,
    })
}

/// The agent-side gate shared by register/poll/io/result/close:
/// verify the JWT, then bind its claims to the session row — the
/// repository and workflow must be the ones the seat opened the
/// session for, and once a run is bound the claims must name it.
fn verify_agent(req: &Value, row: &Value) -> Result<OidcClaims, Value> {
    let Some(jwt) = req_str(req, "_oidc_jwt").filter(|s| !s.is_empty()) else {
        return Err(fail(4, "agent auth required (OIDC JWT)".to_string()));
    };
    let claims = verify_oidc(jwt).map_err(|e| fail(4, format!("oidc: {e}")))?;
    let host_repo = row.get("host_repo").and_then(|v| v.as_str()).unwrap_or("");
    let workflow = row.get("workflow").and_then(|v| v.as_str()).unwrap_or("");
    if claims.repository != host_repo {
        return Err(fail(
            4,
            format!(
                "oidc repository {} does not match session host {}",
                claims.repository, host_repo
            ),
        ));
    }
    let expect_prefix = format!("{host_repo}/.github/workflows/{workflow}@");
    if !claims.job_workflow_ref.starts_with(&expect_prefix) {
        return Err(fail(
            4,
            format!(
                "oidc workflow {} is not the session workflow",
                claims.job_workflow_ref
            ),
        ));
    }
    if let Some(bound) = row.get("run_id").and_then(|v| v.as_i64()) {
        if bound != claims.run_id {
            return Err(fail(4, "oidc run does not match the bound run".to_string()));
        }
        if let Some(bound_attempt) = row.get("run_attempt").and_then(|v| v.as_i64()) {
            if bound_attempt != claims.run_attempt {
                return Err(fail(
                    4,
                    "oidc run attempt does not match the bound attempt".to_string(),
                ));
            }
        }
    }
    Ok(claims)
}

// ---------------------------------------------------------------------------
// Seat verbs
// ---------------------------------------------------------------------------

pub(crate) fn verb_actions_exec_open(t: &TursoBackend, req: &Value) -> Value {
    let Some(host_repo) = req_str(req, "host_repo").filter(|s| s.contains('/')) else {
        return fail(2, "host_repo (owner/repository) is required".to_string());
    };
    let workflow = req_str(req, "workflow")
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WORKFLOW)
        .to_string();
    let seat = req_str(req, "seat")
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown-seat")
        .to_string();
    // Optional session context (a task/issue link the seat names
    // for the session): carried into extension pushes so Drew can
    // see where a session comes from. Free-form, length-capped.
    let context = req_str(req, "context")
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(300).collect::<String>());
    let lease_s = req_i64(req, "lease_s").unwrap_or(0);
    let max_s = req_i64(req, "max_s").unwrap_or(DEFAULT_MAX_S);
    if lease_s < MIN_LEASE_S {
        return fail(
            2,
            format!(
                "lease_s must be at least {MIN_LEASE_S} (the lease is the start --timeout; there is no default)"
            ),
        );
    }
    if max_s > MAX_LIFESPAN_S {
        return fail(
            2,
            format!("max_s must not exceed {MAX_LIFESPAN_S} (the fleet leg cap)"),
        );
    }
    if lease_s > max_s {
        return fail(2, "lease_s must not exceed max_s".to_string());
    }
    let handle = req_str(req, "handle")
        .filter(|h| {
            h.len() >= 8
                && h.len() <= 64
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("fx-{}", rand_hex("handle", 10)));
    let hide_logs = if req.get("hide_logs").and_then(|v| v.as_bool()) == Some(true) {
        1
    } else {
        0
    };
    let now = now_unix();
    let lease_until = iso(now + lease_s);
    let max_until = iso(now + max_s);
    let stamp = iso(now);
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "INSERT INTO actions_exec_sessions \
                (handle, host_repo, workflow, seat, state, hide_logs, \
                 lease_until, max_until, context, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'open', ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(handle.clone()),
                Arg::Text(host_repo.to_string()),
                Arg::Text(workflow),
                Arg::Text(seat),
                Arg::Int(hide_logs),
                Arg::Text(lease_until.clone()),
                Arg::Text(max_until.clone()),
                match &context {
                    Some(c) => Arg::Text(c.clone()),
                    None => Arg::Null,
                },
                Arg::Text(stamp.clone()),
                Arg::Text(stamp),
            ],
        )?;
        Ok(json!({
            "handle": handle,
            "state": "open",
            "lease_until": lease_until,
            "max_until": max_until,
            "context": context,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_bind(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let (Some(run_id), Some(run_attempt)) = (req_i64(req, "run_id"), req_i64(req, "run_attempt"))
    else {
        return fail(2, "run_id and run_attempt are required".to_string());
    };
    let work = (|| -> Result<Value, TErr> {
        let res = t.exec(
            "UPDATE actions_exec_sessions \
             SET run_id = ?, run_attempt = ?, state = 'bound', updated_at = ? \
             WHERE handle = ? AND state IN ('open', 'bound')",
            &[
                Arg::Int(run_id),
                Arg::Int(run_attempt),
                Arg::Text(now_iso()),
                Arg::Text(handle.to_string()),
            ],
        )?;
        if res.affected == 0 {
            return Ok(json!({"bound": false}));
        }
        Ok(json!({"bound": true, "run_id": run_id, "run_attempt": run_attempt}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_command(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let now = now_unix();
    let state = effective_state(&row, now);
    if !matches!(state.as_str(), "bound" | "live" | "extension-pending") {
        return fail(
            5,
            format!("session is {state}; commands need a bound or live session"),
        );
    }
    let Some(text) = req_str(req, "text").filter(|s| !s.is_empty()) else {
        return fail(2, "text (the command) is required".to_string());
    };
    let timeout_s = req_i64(req, "timeout_s").unwrap_or(600).clamp(1, 3_600);
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let cmd_id = format!("cmd-{}", rand_hex(handle, 8));
    let stamp = now_iso();
    let work = (|| -> Result<Value, TErr> {
        let seq_rows = t.query_objs(
            "SELECT COALESCE(MAX(seq), 0) + 1 AS next_seq \
             FROM actions_exec_commands WHERE handle = ?",
            &[Arg::Text(handle.to_string())],
            &[],
        )?;
        let seq = seq_rows
            .first()
            .and_then(|r| r.get("next_seq"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        t.exec(
            "INSERT INTO actions_exec_commands \
                (cmd_id, handle, seq, text, timeout_s, state, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'queued', ?, ?)",
            &[
                Arg::Text(cmd_id.clone()),
                Arg::Text(handle.to_string()),
                Arg::Int(seq),
                Arg::Text(text.to_string()),
                Arg::Int(timeout_s),
                Arg::Text(stamp.clone()),
                Arg::Text(stamp),
            ],
        )?;
        Ok(json!({"cmd_id": cmd_id, "seq": seq, "state": "queued"}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_stream(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session_settled(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let after_id = req_i64(req, "after_id").unwrap_or(0);
    let cmd_filter = req_str(req, "cmd_id").map(str::to_string);
    let limit = req_i64(req, "limit").unwrap_or(500).clamp(1, 2_000);
    let now = now_unix();
    let work = (|| -> Result<Value, TErr> {
        let (sql, args): (&str, Vec<Arg>) = match &cmd_filter {
            Some(cmd) => (
                "SELECT id, cmd_id, seq, stream, data FROM actions_exec_io \
                 WHERE handle = ? AND cmd_id = ? AND id > ? ORDER BY id LIMIT ?",
                vec![
                    Arg::Text(handle.to_string()),
                    Arg::Text(cmd.clone()),
                    Arg::Int(after_id),
                    Arg::Int(limit),
                ],
            ),
            None => (
                "SELECT id, cmd_id, seq, stream, data FROM actions_exec_io \
                 WHERE handle = ? AND id > ? ORDER BY id LIMIT ?",
                vec![
                    Arg::Text(handle.to_string()),
                    Arg::Int(after_id),
                    Arg::Int(limit),
                ],
            ),
        };
        let chunks = t.query_objs(sql, &args, &[])?;
        let cmds = t.query_objs(
            "SELECT cmd_id, seq, text, timeout_s, state, exit_code \
             FROM actions_exec_commands WHERE handle = ? ORDER BY seq",
            &[Arg::Text(handle.to_string())],
            &[],
        )?;
        let mut session = session_json(&row);
        session["state"] = json!(effective_state(&row, now));
        session["server_now"] = json!(now);
        Ok(json!({
            "session": session,
            "chunks": chunks,
            "commands": cmds,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_renew(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session_settled(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let now = now_unix();
    let state = effective_state(&row, now);
    if !matches!(
        state.as_str(),
        "open" | "bound" | "live" | "extension-pending"
    ) {
        return fail(5, format!("session is {state}; it cannot be renewed"));
    }
    let Some(lease_s) = req_i64(req, "lease_s") else {
        return fail(
            2,
            "lease_s is required (renewal names its timeout)".to_string(),
        );
    };
    if lease_s < 1 {
        return fail(2, "lease_s must be positive".to_string());
    }
    let max_unix = row
        .get("max_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let target = now + lease_s;
    let extension_required = target > max_unix;
    let new_lease_unix = target.min(max_unix);
    if new_lease_unix <= now {
        return fail(
            5,
            "session is at its max lifespan; an approved emergency extension is the only way past it"
                .to_string(),
        );
    }
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let new_lease = iso(new_lease_unix);
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE actions_exec_sessions SET lease_until = ?, updated_at = ? \
             WHERE handle = ? AND state IN ('open', 'bound', 'live', 'extension-pending')",
            &[
                Arg::Text(new_lease.clone()),
                Arg::Text(now_iso()),
                Arg::Text(handle.to_string()),
            ],
        )?;
        Ok(json!({
            "lease_until": new_lease,
            "max_until": row.get("max_until").cloned().unwrap_or(Value::Null),
            "extension_required": extension_required,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_extend_request(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session_settled(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let now = now_unix();
    let state = effective_state(&row, now);
    if !matches!(
        state.as_str(),
        "open" | "bound" | "live" | "extension-pending"
    ) {
        return fail(
            5,
            format!("session is {state}; extensions need a live session"),
        );
    }
    if row.get("ext_state").and_then(|v| v.as_str()) == Some("pending") {
        return fail(5, "an extension request is already pending".to_string());
    }
    // The tier ladder reads the RAW ask: an oversized request is
    // a manual-tier request even though the grant itself stays
    // clamped to the per-approval bound.
    let raw_extend_s = req_i64(req, "extend_s").unwrap_or(MAX_EXTENSION_S);
    let extend_s = raw_extend_s.clamp(60, MAX_EXTENSION_S);
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let count_before = ext_count(&row);
    let total_before = ext_total_s(&row);
    let tier = evaluate_extension_tier(count_before, total_before, raw_extend_s, extend_s);
    let ordinal = count_before + 1;
    let lease_unix = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let max_unix = row
        .get("max_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    let created_unix = row
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(0);
    // Everything queue_broker's alert builder needs, on every
    // tier: who, where from, the run, the lease state, the ask,
    // the cumulative counters (before this request), the tier.
    let mut out = json!({
        "requested": true,
        "handle": handle,
        "host_repo": row.get("host_repo").cloned().unwrap_or(Value::Null),
        "workflow": row.get("workflow").cloned().unwrap_or(Value::Null),
        "seat": row.get("seat").cloned().unwrap_or(Value::Null),
        "context": row.get("context").cloned().unwrap_or(Value::Null),
        "run_id": row.get("run_id").cloned().unwrap_or(Value::Null),
        "run_attempt": row.get("run_attempt").cloned().unwrap_or(Value::Null),
        "created_at": row.get("created_at").cloned().unwrap_or(Value::Null),
        "granted_s": max_unix - created_unix,
        "elapsed_s": now - created_unix,
        "extend_s": extend_s,
        "tier": tier,
        "ext_ordinal": ordinal,
        "alert_key": format!("actions-exec-ext:{handle}:{ordinal}"),
        "ext_count": count_before,
        "ext_total_s": total_before,
        "reason": reason.clone(),
    });
    let work = (|| -> Result<Value, TErr> {
        match tier {
            1 => {
                // AUTO + NOTIFY: apply in this request, with the
                // same grant shape a manual approval produces.
                let grant = ExtensionGrant {
                    extend_s,
                    tier,
                    reason: reason.clone(),
                    outcome: "auto-approved",
                };
                match apply_extension(t, &row, now, &grant, false)? {
                    Some((new_max, new_lease)) => {
                        out["outcome"] = json!("auto-approved");
                        out["max_until"] = json!(new_max);
                        out["lease_until"] = json!(new_lease.clone());
                        out["grace_until"] = json!(new_lease);
                        Ok(out)
                    }
                    None => Ok(json!({"requested": false})),
                }
            }
            _ => {
                let decide_after = if tier == 2 {
                    Some(iso(now + EXT_DENY_WINDOW_S))
                } else {
                    None
                };
                let res = t.exec(
                    "UPDATE actions_exec_sessions \
                     SET state = 'extension-pending', ext_state = 'pending', \
                         ext_seconds = ?, ext_tier = ?, ext_reason = ?, \
                         ext_decide_after = ?, updated_at = ? \
                     WHERE handle = ? AND state IN ('open', 'bound', 'live', 'extension-pending') \
                       AND COALESCE(ext_state, '') <> 'pending'",
                    &[
                        Arg::Int(extend_s),
                        Arg::Int(tier),
                        Arg::Text(reason.clone()),
                        match &decide_after {
                            Some(s) => Arg::Text(s.clone()),
                            None => Arg::Null,
                        },
                        Arg::Text(now_iso()),
                        Arg::Text(handle.to_string()),
                    ],
                )?;
                if res.affected == 0 {
                    return Ok(json!({"requested": false}));
                }
                out["outcome"] = json!(if tier == 2 {
                    "pending-deny-window"
                } else {
                    "pending-manual"
                });
                out["max_until"] = row.get("max_until").cloned().unwrap_or(Value::Null);
                out["lease_until"] = row.get("lease_until").cloned().unwrap_or(Value::Null);
                out["decide_after"] = match &decide_after {
                    Some(s) => json!(s),
                    None => Value::Null,
                };
                out["grace_until"] = json!(iso(if tier == 2 {
                    lease_unix.max(now + EXT_DENY_WINDOW_S) + EXTENSION_GRACE_S
                } else {
                    lease_unix + EXTENSION_GRACE_S
                }));
                Ok(out)
            }
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_extend_decide(t: &TursoBackend, req: &Value) -> Value {
    verb_actions_exec_extend_decide_with(t, req, &resolve_apply_key)
}

/// [`verb_actions_exec_extend_decide`] with the apply-key resolver
/// injected: production resolves the named secret through
/// fleet-secrets; tests inject a fixed key so no test touches the
/// secrets chain or the network.
pub(crate) fn verb_actions_exec_extend_decide_with(
    t: &TursoBackend,
    req: &Value,
    resolve_key: &dyn Fn() -> Result<Vec<u8>, String>,
) -> Value {
    let row = match require_session_settled(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    // A signed applier envelope authenticates before the state
    // machine reads: a bad signature or a stale envelope is an
    // auth refusal whatever the session state, while a valid
    // envelope flows through the same terminal outcomes a seat
    // sees (silence-approved / settled / no-pending), which the
    // applier reads as superseded/rejected. Any presence of the
    // `sig` field selects the signed path; without it the call is
    // a seat call with byte-identical behaviour to before.
    let via_signed = req.get("sig").is_some();
    if via_signed {
        let key = match resolve_key() {
            Ok(k) => k,
            Err(e) => return fail(4, format!("signed decide unavailable: {e}")),
        };
        if let Err(v) = check_signed_envelope(req, &key, now_unix()) {
            return v;
        }
    }
    let via = if via_signed { "signed-apply" } else { "seat" };
    // A tier-2 request whose deny window lapsed was settled by
    // require_session_settled: nothing is left to decide, and a
    // late deny must not read as if it landed.
    if row.get("ext_state").and_then(|v| v.as_str()) == Some("silence-approved") {
        return ok(json!({
            "decided": false,
            "approved": true,
            "tier": 2,
            "outcome": "silence-approved",
            "via": via,
            "note": "the deny window lapsed before this decision arrived; the extension was auto-approved at window end",
        }));
    }
    if row.get("ext_state").and_then(|v| v.as_str()) != Some("pending") {
        return fail(
            5,
            "no extension request is pending on this session".to_string(),
        );
    }
    let approve = req.get("approve").and_then(|v| v.as_bool()) == Some(true);
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let now = now_unix();
    let tier = row.get("ext_tier").and_then(|v| v.as_i64()).unwrap_or(3);
    let work = (|| -> Result<Value, TErr> {
        if approve {
            let grant = ExtensionGrant {
                extend_s: row
                    .get("ext_seconds")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(MAX_EXTENSION_S)
                    .clamp(60, MAX_EXTENSION_S),
                tier,
                reason: row
                    .get("ext_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                outcome: "approved",
            };
            match apply_extension(t, &row, now, &grant, true)? {
                Some((new_max, new_lease)) => Ok(json!({
                    "decided": true,
                    "approved": true,
                    "tier": tier,
                    "outcome": "approved",
                    "via": via,
                    "max_until": new_max,
                    "lease_until": new_lease,
                })),
                None => Ok(json!({"decided": false, "approved": true, "via": via})),
            }
        } else {
            let res = t.exec(
                "UPDATE actions_exec_sessions \
                 SET state = 'live', ext_state = 'denied', ext_decide_after = NULL, updated_at = ? \
                 WHERE handle = ? AND ext_state = 'pending'",
                &[Arg::Text(now_iso()), Arg::Text(handle.to_string())],
            )?;
            Ok(json!({
                "decided": res.affected > 0,
                "approved": false,
                "tier": tier,
                "outcome": "denied",
                "via": via,
            }))
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_close(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    // Dual-sided: a seat close arrives behind the broker token (no
    // _oidc_jwt); an agent close must prove itself like any agent
    // verb.
    if req_str(req, "_oidc_jwt").is_some() {
        if let Err(v) = verify_agent(req, &row) {
            return v;
        }
    }
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let work = (|| -> Result<Value, TErr> {
        let res = t.exec(
            "UPDATE actions_exec_sessions \
             SET state = 'closed', closed_at = ?, updated_at = ? \
             WHERE handle = ? AND state <> 'closed'",
            &[
                Arg::Text(now_iso()),
                Arg::Text(now_iso()),
                Arg::Text(handle.to_string()),
            ],
        )?;
        t.exec(
            "UPDATE actions_exec_commands SET state = 'error', updated_at = ? \
             WHERE handle = ? AND state = 'queued'",
            &[Arg::Text(now_iso()), Arg::Text(handle.to_string())],
        )?;
        Ok(json!({"closed": res.affected > 0}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Agent verbs
// ---------------------------------------------------------------------------

pub(crate) fn verb_actions_exec_register(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let claims = match verify_agent(req, &row) {
        Ok(c) => c,
        Err(v) => return v,
    };
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let work = (|| -> Result<Value, TErr> {
        // The verified claims bind the run when the seat has not
        // bound it yet; a bound run was already required to match
        // by verify_agent.
        let res = t.exec(
            "UPDATE actions_exec_sessions \
             SET state = 'live', \
                 run_id = COALESCE(run_id, ?), run_attempt = COALESCE(run_attempt, ?), \
                 updated_at = ? \
             WHERE handle = ? AND state IN ('open', 'bound', 'live')",
            &[
                Arg::Int(claims.run_id),
                Arg::Int(claims.run_attempt),
                Arg::Text(now_iso()),
                Arg::Text(handle.to_string()),
            ],
        )?;
        if res.affected == 0 {
            return Ok(json!({"registered": false}));
        }
        Ok(json!({
            "registered": true,
            "state": "live",
            "lease_until": row.get("lease_until").cloned().unwrap_or(Value::Null),
            "max_until": row.get("max_until").cloned().unwrap_or(Value::Null),
            "hide_logs": row.get("hide_logs").and_then(|v| v.as_i64()).unwrap_or(0) != 0,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_poll(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    if let Err(v) = verify_agent(req, &row) {
        return v;
    }
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let wait_s = req_i64(req, "wait_s")
        .unwrap_or(0)
        .clamp(0, MAX_POLL_WAIT_S);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_s as u64);
    // Long-poll: re-read the session + queued commands until a
    // command appears, the session leaves a live shape, or the
    // wait budget runs out.
    loop {
        let current = match session_row(t, handle) {
            Ok(Some(r)) => r,
            Ok(None) => return fail(5, format!("unknown session handle {handle}")),
            Err(e) => return terr(e),
        };
        let now = now_unix();
        // A lapsed tier-2 deny window settles here first (silence
        // approves), so the state read below already reflects the
        // grant — this poll is the session's most frequent touch.
        let current = match settle_extension(t, &current, now) {
            Ok(true) => session_row(t, handle).ok().flatten().unwrap_or(current),
            _ => current,
        };
        let state = effective_state(&current, now);
        if state == "expired" && current.get("state").and_then(|v| v.as_str()) != Some("expired") {
            // Write the lazy transition back so every reader agrees.
            let _ = t.exec(
                "UPDATE actions_exec_sessions SET state = 'expired', updated_at = ? \
                 WHERE handle = ? AND state IN ('open', 'bound', 'live', 'extension-pending')",
                &[Arg::Text(now_iso()), Arg::Text(handle.to_string())],
            );
        }
        let queued = match t.query_objs(
            "SELECT cmd_id, seq, text, timeout_s FROM actions_exec_commands \
             WHERE handle = ? AND state = 'queued' ORDER BY seq LIMIT 16",
            &[Arg::Text(handle.to_string())],
            &[],
        ) {
            Ok(q) => q,
            Err(e) => return terr(e),
        };
        if !queued.is_empty() || state != "live" && state != "extension-pending" || wait_s == 0 {
            let mut session = session_json(&current);
            session["state"] = json!(state);
            session["server_now"] = json!(now);
            session["deadline_unix"] = json!(hard_deadline_unix(&current));
            return ok(json!({"session": session, "commands": queued}));
        }
        if std::time::Instant::now() >= deadline {
            let mut session = session_json(&current);
            session["state"] = json!(state);
            session["server_now"] = json!(now);
            session["deadline_unix"] = json!(hard_deadline_unix(&current));
            return ok(json!({"session": session, "commands": queued}));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

pub(crate) fn verb_actions_exec_io(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    if let Err(v) = verify_agent(req, &row) {
        return v;
    }
    let (Some(cmd_id), Some(seq), Some(stream), Some(data)) = (
        req_str(req, "cmd_id"),
        req_i64(req, "seq"),
        req_str(req, "stream"),
        req_str(req, "data"),
    ) else {
        return fail(2, "cmd_id, seq, stream and data are required".to_string());
    };
    if data.len() > MAX_IO_CHUNK {
        return fail(2, "io chunk exceeds the 64 KiB cap".to_string());
    }
    if stream != "stdout" && stream != "stderr" {
        return fail(2, "stream must be stdout or stderr".to_string());
    }
    let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("");
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "INSERT INTO actions_exec_io (handle, cmd_id, seq, stream, data, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(handle.to_string()),
                Arg::Text(cmd_id.to_string()),
                Arg::Int(seq),
                Arg::Text(stream.to_string()),
                Arg::Text(data.to_string()),
                Arg::Text(now_iso()),
            ],
        )?;
        // First IO on a queued command marks it running.
        t.exec(
            "UPDATE actions_exec_commands SET state = 'running', updated_at = ? \
             WHERE cmd_id = ? AND state = 'queued'",
            &[Arg::Text(now_iso()), Arg::Text(cmd_id.to_string())],
        )?;
        Ok(json!({"appended": true}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_actions_exec_result(t: &TursoBackend, req: &Value) -> Value {
    let row = match require_session(t, req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    if let Err(v) = verify_agent(req, &row) {
        return v;
    }
    let (Some(cmd_id), Some(exit_code)) = (req_str(req, "cmd_id"), req_i64(req, "exit_code"))
    else {
        return fail(2, "cmd_id and exit_code are required".to_string());
    };
    let work = (|| -> Result<Value, TErr> {
        let res = t.exec(
            "UPDATE actions_exec_commands \
             SET state = 'done', exit_code = ?, updated_at = ? \
             WHERE cmd_id = ? AND state IN ('queued', 'running')",
            &[
                Arg::Int(exit_code),
                Arg::Text(now_iso()),
                Arg::Text(cmd_id.to_string()),
            ],
        )?;
        Ok(json!({"recorded": res.affected > 0}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Tests: a rusqlite-backed fake of the Turso pipeline endpoint (the
// #13765 proof-bar pattern — real SQL semantics, so the conditional
// UPDATEs and lease transitions are exercised, not mimicked), plus
// a throwaway RSA keypair's signed JWT for the OIDC verbs. The
// private key never left the generating machine and the fixture
// verifies only against the test JWKS below — GitHub's real JWKS
// has no such kid.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::Arc;

    const TEST_JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2lkLTEiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL3Rva2VuLmFjdGlvbnMuZ2l0aHVidXNlcmNvbnRlbnQuY29tIiwic3ViIjoicmVwbzpkZXpyZW4zOS9mbGVldDpyZWY6cmVmcy9oZWFkcy9tYWluIiwiYXVkIjoiaHR0cHM6Ly9naXRodWIuY29tL2RlenJlbjM5IiwiaWF0IjoxNzAwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDAsInJlcG9zaXRvcnkiOiJkZXpyZW4zOS9mbGVldCIsInJlcG9zaXRvcnlfb3duZXIiOiJkZXpyZW4zOSIsIndvcmtmbG93IjoiZmxlZXQtZXhlYyIsIndvcmtmbG93X3JlZiI6ImRlenJlbjM5L2ZsZWV0Ly5naXRodWIvd29ya2Zsb3dzL2ZsZWV0LWV4ZWMueW1sQHJlZnMvaGVhZHMvbWFpbiIsImpvYl93b3JrZmxvd19yZWYiOiJkZXpyZW4zOS9mbGVldC8uZ2l0aHViL3dvcmtmbG93cy9mbGVldC1leGVjLnltbEByZWZzL2hlYWRzL21haW4iLCJydW5faWQiOiIxMjM0NTY3ODkiLCJydW5fYXR0ZW1wdCI6IjEiLCJldmVudF9uYW1lIjoid29ya2Zsb3dfZGlzcGF0Y2giLCJyZWYiOiJyZWZzL2hlYWRzL21haW4iLCJzaGEiOiI1ZDI4ZjAzZGY0MiJ9.dGPmPh4__6ksSCFlT0I7IClcTiGwKoeLexHGEkCkOjjrmSQzkDPQqLT_Sn2dZbZ_5TIt97FhJszmO7B2J9u707iGs8ts1D6H1ZTtpt17IkD68580MlA37IeRM5svsHTCOD56xvnPggJSJ7NkUTLpYQFPDePBjyUI-GG7N_o12OSMuNj7DBP1Uw7fUwi7ZytRLeNANxNN31T-bKsoXNKZ2Y36pr7hk9p3zo9C18iosVNwAARdrK5CPyG0_x499X7x1_qiXJwuMkfIXys9PfeHrH5GgjnPUVo9IZSC_AvdPdC0aWMpwLwr5gZNTisRRWqAfzgNbB3skeync8brxTCalg";
    const TEST_JWK_N: &str = "v-HGaXBUMPAM-kNMOETfcHvPUX-Rx0-mXHqK_KV6-1HuhDeCc2Tg2iD0z136S19t1-CIGvMvQyiKGIV5vUlrs1b7SIgh-YEbLNtESJr6soNpONOtIL_eH1ZI4WrL8RKmkK0OZpa4BRKyBooA2qIMDmcwa4Gl9Ae0wIgrRyHk0DnxLf2uTC3t8UtHvFPiM82pOv9DC3RpvMCtOay4-97P3T1Ir5mCMwRKex5KNApSGXmvTCfx6ALM7A7EAzi9-nRW_AlUPdiNDIh1qXLX8FG0Mw6OGOAyBFsbQ5tEc159dP74NB15AJeCTCW9gObJCc4jgXTWO5z9uQ3b81fnYLHX9Q";
    const TEST_JWK_E: &str = "AQAB";

    /// One shared fake store for the whole module (tests use unique
    /// handles, so they cannot collide), serving both the pipeline
    /// endpoint and the test JWKS.
    fn fake_store() -> &'static (String, Arc<Mutex<rusqlite::Connection>>) {
        static STORE: std::sync::OnceLock<(String, Arc<Mutex<rusqlite::Connection>>)> =
            std::sync::OnceLock::new();
        STORE.get_or_init(|| {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            for ddl in [
                ACTIONS_EXEC_DDL_SESSIONS,
                ACTIONS_EXEC_DDL_COMMANDS,
                ACTIONS_EXEC_DDL_COMMANDS_IDX,
                ACTIONS_EXEC_DDL_IO,
                ACTIONS_EXEC_DDL_IO_IDX,
            ] {
                conn.execute_batch(ddl).unwrap();
            }
            let db = Arc::new(Mutex::new(conn));
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            // SAFETY: this OnceLock initializer runs once, before
            // any test can read the variable, and the only reader
            // is this module's OIDC path.
            unsafe {
                std::env::set_var("FLEET_ACTIONS_EXEC_JWKS_URL", format!("{base}/jwks"));
            }
            let db_thread = db.clone();
            std::thread::spawn(move || {
                for sock in listener.incoming() {
                    let Ok(sock) = sock else { return };
                    let db_conn = db_thread.clone();
                    std::thread::spawn(move || handle_conn(sock, db_conn));
                }
            });
            (base, db)
        })
    }

    fn cell_json(v: rusqlite::types::ValueRef) -> Value {
        use rusqlite::types::ValueRef::*;
        match v {
            Null => json!({"type": "null"}),
            Integer(i) => json!({"type": "integer", "value": i.to_string()}),
            Real(f) => json!({"type": "float", "value": f}),
            Text(s) => json!({"type": "text", "value": String::from_utf8_lossy(s)}),
            Blob(b) => json!({"type": "blob", "value": b64_encode(b)}),
        }
    }

    fn b64_encode(data: &[u8]) -> String {
        const TBL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(TBL[(n >> 18) as usize & 63] as char);
            out.push(TBL[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                TBL[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TBL[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    fn handle_conn(mut sock: std::net::TcpStream, db: Arc<Mutex<rusqlite::Connection>>) {
        let mut reader = BufReader::new(sock.try_clone().unwrap());
        loop {
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                return;
            }
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some((name, value)) = trimmed.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            let (status, payload) = if request_line.contains("/jwks") {
                (
                    200,
                    json!({"keys": [{"kty": "RSA", "kid": "test-kid-1", "use": "sig",
                                     "alg": "RS256", "n": TEST_JWK_N, "e": TEST_JWK_E}]}),
                )
            } else {
                (200, run_pipeline(&db, &body))
            };
            let text = payload.to_string();
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{}",
                text.len(),
                text
            );
            if sock.write_all(resp.as_bytes()).is_err() {
                return;
            }
        }
    }

    fn run_pipeline(db: &Arc<Mutex<rusqlite::Connection>>, body: &[u8]) -> Value {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let conn = db.lock().unwrap();
        let mut results = Vec::new();
        for r in req
            .get("requests")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            match r.get("type").and_then(|v| v.as_str()) {
                Some("close") => {
                    results.push(json!({"type": "ok", "response": {"type": "close"}}));
                }
                Some("execute") => {
                    let stmt = r.get("stmt").cloned().unwrap_or(Value::Null);
                    let sql = stmt.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                    let mut params: Vec<rusqlite::types::Value> = Vec::new();
                    if let Some(args) = stmt.get("args").and_then(|v| v.as_array()) {
                        for a in args {
                            params.push(match a.get("type").and_then(|v| v.as_str()) {
                                Some("text") => rusqlite::types::Value::Text(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                ),
                                Some("integer") => rusqlite::types::Value::Integer(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0),
                                ),
                                _ => rusqlite::types::Value::Null,
                            });
                        }
                    }
                    results.push(execute_sql(&conn, sql, &params));
                }
                _ => {}
            }
        }
        json!({"results": results, "baton": null})
    }

    fn execute_sql(
        conn: &rusqlite::Connection,
        sql: &str,
        params: &[rusqlite::types::Value],
    ) -> Value {
        let is_query = sql.trim_start().to_ascii_uppercase().starts_with("SELECT");
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
            }
        };
        if is_query {
            let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let mut rows_out = Vec::new();
            match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(mut rows) => loop {
                    match rows.next() {
                        Ok(Some(row)) => {
                            let mut cells = Vec::new();
                            for i in 0..cols.len() {
                                cells.push(cell_json(row.get_ref(i).unwrap()));
                            }
                            rows_out.push(Value::Array(cells));
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                        }
                    }
                },
                Err(e) => {
                    return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                }
            }
            json!({"type": "ok", "response": {"type": "execute", "result": {
                "cols": cols.iter().map(|c| json!({"name": c, "decltype": null})).collect::<Vec<_>>(),
                "rows": rows_out, "affected_row_count": 0, "last_insert_rowid": null}}})
        } else {
            match stmt.execute(rusqlite::params_from_iter(params.iter())) {
                Ok(changed) => json!({"type": "ok", "response": {"type": "execute", "result": {
                    "cols": [], "rows": [], "affected_row_count": changed,
                    "last_insert_rowid": conn.last_insert_rowid().to_string()}}}),
                Err(e) => {
                    json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                }
            }
        }
    }

    fn backend() -> TursoBackend {
        let (base, _) = fake_store();
        TursoBackend::new(base.clone(), "test".to_string(), ureq::agent())
    }

    fn open_session(t: &TursoBackend, handle: &str, lease_s: i64, max_s: i64) -> Value {
        verb_actions_exec_open(
            t,
            &json!({
                "handle": handle, "host_repo": "dezren39/fleet",
                "seat": "test-seat", "lease_s": lease_s, "max_s": max_s,
            }),
        )
    }

    #[test]
    fn iso_parse_round_trips() {
        for unix in [0, 1_700_000_000, 1_799_999_999, 4_102_444_800] {
            assert_eq!(parse_iso(&iso(unix)), Some(unix), "unix {unix}");
        }
        assert_eq!(parse_iso("not a date"), None);
        assert_eq!(parse_iso("2026-11-23 15:00:00"), None);
    }

    #[test]
    fn open_validates_the_lease_contract() {
        let t = backend();
        // No lease at all: the lease is required, there is no default.
        let out = verb_actions_exec_open(
            &t,
            &json!({"host_repo": "dezren39/fleet", "seat": "s", "max_s": 1800}),
        );
        assert_eq!(out["ok"], false, "{out}");
        // Lease past the max, and a max past the 2 h leg cap.
        let out = open_session(&t, "fx-test-open1", 900, 600);
        assert_eq!(out["ok"], false, "{out}");
        let out = open_session(&t, "fx-test-open2", 900, 7_201);
        assert_eq!(out["ok"], false, "{out}");
        // A valid open names its lease and max.
        let out = open_session(&t, "fx-test-open3", 900, 1_800);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["handle"], "fx-test-open3");
        assert_eq!(out["state"], "open");
    }

    #[test]
    fn seat_flow_bind_command_stream_close() {
        let t = backend();
        let h = "fx-test-flow1";
        assert_eq!(open_session(&t, h, 900, 1_800)["ok"], true);
        let out = verb_actions_exec_bind(&t, &json!({"handle": h, "run_id": 42, "run_attempt": 2}));
        assert_eq!(out["bound"], true, "{out}");
        let cmd = verb_actions_exec_command(
            &t,
            &json!({"handle": h, "text": "echo hello", "timeout_s": 30}),
        );
        assert_eq!(cmd["ok"], true, "{cmd}");
        assert_eq!(cmd["seq"], 1);
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["state"], "bound", "{stream}");
        assert_eq!(stream["commands"][0]["text"], "echo hello");
        assert_eq!(stream["commands"][0]["state"], "queued");
        // Close: queued commands are errored, the session is closed,
        // and new commands are refused.
        let out = verb_actions_exec_close(&t, &json!({"handle": h}));
        assert_eq!(out["closed"], true, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["state"], "closed");
        assert_eq!(stream["commands"][0]["state"], "error");
        let out = verb_actions_exec_command(&t, &json!({"handle": h, "text": "echo nope"}));
        assert_eq!(out["ok"], false, "{out}");
    }

    #[test]
    fn renew_clamps_at_max_and_names_the_extension() {
        let t = backend();
        let h = "fx-test-renew";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        // Ask for far more than the max: clamped, extension flagged.
        let out = verb_actions_exec_renew(&t, &json!({"handle": h, "lease_s": 3600}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["extension_required"], true, "{out}");
        let lease_unix = parse_iso(out["lease_until"].as_str().unwrap()).unwrap();
        let max_unix = parse_iso(out["max_until"].as_str().unwrap()).unwrap();
        assert_eq!(
            lease_unix, max_unix,
            "the clamped lease lands exactly on the max"
        );
        // Once the max is in the past, renewal is refused outright.
        t.exec(
            "UPDATE actions_exec_sessions SET max_until = ?, lease_until = ? WHERE handle = ?",
            &[
                Arg::Text(iso(now_unix() - 10)),
                Arg::Text(iso(now_unix() - 10)),
                Arg::Text(h.to_string()),
            ],
        )
        .unwrap();
        let out = verb_actions_exec_renew(&t, &json!({"handle": h, "lease_s": 600}));
        assert_eq!(out["ok"], false, "{out}");
    }

    #[test]
    fn extension_request_and_decision_move_the_max() {
        // Oracle: the tier-3 (manual) path — an oversized raw ask
        // is manual even as a first extension (the size-cap rule),
        // its grant stays clamped to the per-approval bound, and
        // decide moves the max exactly as v1 did.
        let t = backend();
        let h = "fx-test-extend";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        verb_actions_exec_bind(&t, &json!({"handle": h, "run_id": 7, "run_attempt": 1}));
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 7200}));
        assert_eq!(out["requested"], true, "{out}");
        assert_eq!(out["tier"], 3, "{out}");
        assert_eq!(out["extend_s"], 1800, "{out}");
        assert_eq!(out["run_id"], 7);
        // A second request while one is pending is refused.
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h}));
        assert_eq!(out["ok"], false, "{out}");
        // Deny: state returns to live-shaped, max unmoved, no count.
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": false}));
        assert_eq!(out["approved"], false, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "denied");
        assert_eq!(stream["session"]["ext_count"], 0);
        // Approve on a fresh request: max and lease both move +1800.
        let before = stream["session"]["max_unix"].as_i64().unwrap();
        verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 7200}));
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": true}));
        assert_eq!(out["approved"], true, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(
            stream["session"]["max_unix"].as_i64().unwrap(),
            before + 1800
        );
        assert_eq!(stream["session"]["ext_state"], "approved");
        assert_eq!(stream["session"]["ext_count"], 1);
        assert_eq!(stream["session"]["ext_total_s"], 1800);
    }

    #[test]
    fn tier1_first_extension_auto_approves() {
        // Oracle: policy tier 1 — the first extension within the
        // caps is granted by the request itself: no pending state,
        // the max moves, the counters record it, and the next
        // request is free to pend as tier 2.
        let t = backend();
        let h = "fx-test-tier1";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        verb_actions_exec_bind(&t, &json!({"handle": h, "run_id": 11, "run_attempt": 2}));
        let before = verb_actions_exec_stream(&t, &json!({"handle": h}))["session"]["max_unix"]
            .as_i64()
            .unwrap();
        let out = verb_actions_exec_extend_request(
            &t,
            &json!({"handle": h, "extend_s": 1800, "reason": "build still running"}),
        );
        assert_eq!(out["requested"], true, "{out}");
        assert_eq!(out["tier"], 1, "{out}");
        assert_eq!(out["outcome"], "auto-approved", "{out}");
        assert_eq!(
            out["alert_key"], "actions-exec-ext:fx-test-tier1:1",
            "{out}"
        );
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "auto-approved");
        assert_eq!(stream["session"]["ext_count"], 1);
        assert_eq!(stream["session"]["ext_total_s"], 1800);
        assert_eq!(
            stream["session"]["max_unix"].as_i64().unwrap(),
            before + 1800
        );
        // Not pending: a follow-up request pends as tier 2 rather
        // than being refused as a duplicate.
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 900}));
        assert_eq!(out["requested"], true, "{out}");
        assert_eq!(out["tier"], 2, "{out}");
        assert_eq!(out["outcome"], "pending-deny-window", "{out}");
    }

    #[test]
    fn tier2_silence_approves_at_window_end() {
        // Oracle: policy tier 2 — silence at the deny-window end
        // approves; the settle lands on the next verb that touches
        // the session (the stream here stands in for the agent's
        // poll), and a decision arriving after the lapse is told
        // it changed nothing.
        let t = backend();
        let h = "fx-test-tier2-silence";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        assert_eq!(out["tier"], 2, "{out}");
        assert_eq!(out["outcome"], "pending-deny-window", "{out}");
        assert!(out["decide_after"].as_str().is_some(), "{out}");
        let before = verb_actions_exec_stream(&t, &json!({"handle": h}))["session"]["max_unix"]
            .as_i64()
            .unwrap();
        // Age the window out and touch the session.
        t.exec(
            "UPDATE actions_exec_sessions SET ext_decide_after = ? WHERE handle = ?",
            &[Arg::Text(iso(now_unix() - 1)), Arg::Text(h.to_string())],
        )
        .unwrap();
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "silence-approved");
        assert_eq!(stream["session"]["ext_count"], 2);
        assert_eq!(stream["session"]["ext_total_s"], 3600);
        assert_eq!(
            stream["session"]["max_unix"].as_i64().unwrap(),
            before + 1800
        );
        // A deny arriving after the lapse is told the truth.
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": false}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["decided"], false, "{out}");
        assert_eq!(out["outcome"], "silence-approved", "{out}");
    }

    #[test]
    fn tier2_deny_inside_the_window_ends_it() {
        // Oracle: policy tier 2 — a deny inside the window kills
        // the extension: the session keeps its current cap, the
        // counters do not move, and a later lapse cannot resurrect
        // the denied request.
        let t = backend();
        let h = "fx-test-tier2-deny";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        let capped = verb_actions_exec_stream(&t, &json!({"handle": h}))["session"]["max_unix"]
            .as_i64()
            .unwrap();
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        assert_eq!(out["tier"], 2, "{out}");
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": false}));
        assert_eq!(out["decided"], true, "{out}");
        assert_eq!(out["approved"], false, "{out}");
        // Even with the window aged out, a denied request stays denied.
        t.exec(
            "UPDATE actions_exec_sessions SET ext_decide_after = ? WHERE handle = ?",
            &[Arg::Text(iso(now_unix() - 1)), Arg::Text(h.to_string())],
        )
        .unwrap();
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "denied");
        assert_eq!(stream["session"]["ext_count"], 1);
        assert_eq!(stream["session"]["max_unix"].as_i64().unwrap(), capped);
    }

    #[test]
    fn tier3_third_extension_stays_manual() {
        // Oracle: policy tier 3 — the third extension never
        // auto-settles, however often the session is touched;
        // only a recorded decision moves it.
        let t = backend();
        let h = "fx-test-tier3";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": true}));
        assert_eq!(out["approved"], true, "{out}");
        assert_eq!(out["tier"], 2, "{out}");
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 1800}));
        assert_eq!(out["tier"], 3, "{out}");
        assert_eq!(out["outcome"], "pending-manual", "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "pending");
        assert_eq!(stream["session"]["ext_count"], 2);
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": true}));
        assert_eq!(out["approved"], true, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_count"], 3);
        assert_eq!(stream["session"]["ext_state"], "approved");
    }

    #[test]
    fn cumulative_cap_escalates_to_tier3() {
        // Oracle: the cumulative auto cap — a session whose
        // granted total leaves no room under EXT_AUTO_TOTAL_CAP_S
        // goes manual even for its second extension.
        let t = backend();
        let h = "fx-test-tier-cum";
        assert_eq!(open_session(&t, h, 60, 120)["ok"], true);
        t.exec(
            "UPDATE actions_exec_sessions SET ext_count = 1, ext_total_s = 3500 WHERE handle = ?",
            &[Arg::Text(h.to_string())],
        )
        .unwrap();
        let out = verb_actions_exec_extend_request(&t, &json!({"handle": h, "extend_s": 900}));
        assert_eq!(out["tier"], 3, "{out}");
        assert_eq!(out["outcome"], "pending-manual", "{out}");
    }

    #[test]
    fn deny_window_covers_the_hard_deadline() {
        // Oracle: while a tier-2 request pends, the agent's hard
        // deadline reaches the window end plus grace — the session
        // cannot expire mid-window; a tier-3 request keeps the
        // plain lease + grace.
        let lease = 1_700_000_000;
        let row = json!({
            "lease_until": iso(lease),
            "ext_state": "pending",
            "ext_tier": 2,
            "ext_decide_after": iso(lease + 900),
        });
        assert_eq!(hard_deadline_unix(&row), lease + 900 + EXTENSION_GRACE_S);
        let row = json!({
            "lease_until": iso(lease),
            "ext_state": "pending",
            "ext_tier": 3,
            "ext_decide_after": null,
        });
        assert_eq!(hard_deadline_unix(&row), lease + EXTENSION_GRACE_S);
    }

    #[test]
    fn oidc_agent_flow_register_poll_io_result() {
        let t = backend();
        let h = "fx-test-oidc1";
        assert_eq!(open_session(&t, h, 900, 1_800)["ok"], true);
        // No token: refused before any state change.
        let out = verb_actions_exec_register(&t, &json!({"handle": h}));
        assert_eq!(out["ok"], false, "{out}");
        // A tampered token (signature edited): refused.
        let mut tampered = TEST_JWT.to_string();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        let out = verb_actions_exec_register(&t, &json!({"handle": h, "_oidc_jwt": tampered}));
        assert_eq!(out["ok"], false, "{out}");
        // The real fixture: registers live and binds the run from
        // the verified claims.
        let out = verb_actions_exec_register(&t, &json!({"handle": h, "_oidc_jwt": TEST_JWT}));
        assert_eq!(out["registered"], true, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["state"], "live");
        assert_eq!(stream["session"]["run_id"], 123_456_789);
        assert_eq!(stream["session"]["run_attempt"], 1);
        // Seat enqueues; agent polls it back, streams IO, records
        // the exit code; the seat's stream shows all of it.
        let cmd = verb_actions_exec_command(&t, &json!({"handle": h, "text": "echo hi"}));
        let cmd_id = cmd["cmd_id"].as_str().unwrap().to_string();
        let poll = verb_actions_exec_poll(
            &t,
            &json!({"handle": h, "_oidc_jwt": TEST_JWT, "wait_s": 0}),
        );
        assert_eq!(poll["commands"][0]["cmd_id"], cmd_id, "{poll}");
        let out = verb_actions_exec_io(
            &t,
            &json!({"handle": h, "_oidc_jwt": TEST_JWT, "cmd_id": cmd_id,
                    "seq": 1, "stream": "stdout", "data": "hi\n"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let out = verb_actions_exec_result(
            &t,
            &json!({"handle": h, "_oidc_jwt": TEST_JWT, "cmd_id": cmd_id, "exit_code": 3}),
        );
        assert_eq!(out["recorded"], true, "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["chunks"][0]["data"], "hi\n");
        assert_eq!(stream["commands"][0]["state"], "done");
        assert_eq!(stream["commands"][0]["exit_code"], 3);
        // A session opened for a different host rejects the token:
        // the claims bind to dezren39/fleet, not to the other repo.
        let h2 = "fx-test-oidc2";
        let out = verb_actions_exec_open(
            &t,
            &json!({"handle": h2, "host_repo": "developing-today/code",
                    "seat": "test-seat", "lease_s": 900, "max_s": 1800}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let out = verb_actions_exec_register(&t, &json!({"handle": h2, "_oidc_jwt": TEST_JWT}));
        assert_eq!(out["ok"], false, "{out}");
    }

    #[test]
    fn expired_lease_ends_the_session_on_poll() {
        let t = backend();
        let h = "fx-test-expire";
        assert_eq!(open_session(&t, h, 900, 1_800)["ok"], true);
        let out = verb_actions_exec_register(&t, &json!({"handle": h, "_oidc_jwt": TEST_JWT}));
        assert_eq!(out["registered"], true, "{out}");
        // Drag the lease into the past: the next poll reports the
        // session expired (and writes the transition back).
        t.exec(
            "UPDATE actions_exec_sessions SET lease_until = ? WHERE handle = ?",
            &[Arg::Text(iso(now_unix() - 5)), Arg::Text(h.to_string())],
        )
        .unwrap();
        let poll = verb_actions_exec_poll(
            &t,
            &json!({"handle": h, "_oidc_jwt": TEST_JWT, "wait_s": 0}),
        );
        assert_eq!(poll["session"]["state"], "expired", "{poll}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["state"], "expired");
    }

    // S1 (#14043) oracle helpers: the applier envelope over a
    // fixed test key. Production resolves the named secret through
    // fleet-secrets; tests inject the key so nothing touches the
    // secrets chain or the network.
    fn test_sign(
        key: &[u8],
        handle: &str,
        approve: bool,
        answer_id: &str,
        answered_at: &str,
    ) -> String {
        let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
        let tag = ring::hmac::sign(
            &k,
            signed_decide_message(handle, approve, answer_id, answered_at).as_bytes(),
        );
        tag.as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn pending_manual(t: &TursoBackend, h: &str) {
        assert_eq!(open_session(t, h, 60, 120)["ok"], true);
        verb_actions_exec_bind(t, &json!({"handle": h, "run_id": 1, "run_attempt": 1}));
        let out = verb_actions_exec_extend_request(t, &json!({"handle": h, "extend_s": 7200}));
        assert_eq!(out["tier"], 3, "{out}");
        assert_eq!(out["outcome"], "pending-manual", "{out}");
    }

    #[test]
    fn signed_decide_message_is_canonical() {
        // Pins the cross-language contract: the S3 (aggregator)
        // signer must produce byte-identical input.
        assert_eq!(
            signed_decide_message("fx-h", true, "ans-1", "2026-11-24T05:00:00Z"),
            "fx-h|true|ans-1|2026-11-24T05:00:00Z"
        );
        assert_eq!(
            signed_decide_message("fx-h", false, "ans-2", "2026-11-24T05:00:00Z"),
            "fx-h|false|ans-2|2026-11-24T05:00:00Z"
        );
    }

    #[test]
    fn signed_decide_accepts_a_valid_envelope() {
        let t = backend();
        let key = b"test-apply-key-s1";
        let resolve = || Ok::<Vec<u8>, String>(key.to_vec());
        let h = "fx-test-signed-ok";
        pending_manual(&t, h);
        let before = verb_actions_exec_stream(&t, &json!({"handle": h}))["session"]["max_unix"]
            .as_i64()
            .unwrap();
        let at = now_iso();
        let sig = test_sign(key, h, true, "ans-1", &at);
        let out = verb_actions_exec_extend_decide_with(
            &t,
            &json!({
                "handle": h, "approve": true,
                "answer_id": "ans-1", "answered_at": at, "sig": sig,
            }),
            &resolve,
        );
        assert_eq!(out["approved"], true, "{out}");
        assert_eq!(out["via"], "signed-apply", "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(
            stream["session"]["max_unix"].as_i64().unwrap(),
            before + 1800,
            "{stream}"
        );
        assert_eq!(stream["session"]["ext_state"], "approved");
    }

    #[test]
    fn signed_decide_refuses_a_bad_signature_and_keeps_pending() {
        let t = backend();
        let key = b"test-apply-key-s1";
        let resolve = || Ok::<Vec<u8>, String>(key.to_vec());
        let h = "fx-test-signed-bad";
        pending_manual(&t, h);
        let at = now_iso();
        // Signed under a different key: the envelope is well-formed
        // but the HMAC does not verify.
        let sig = test_sign(b"wrong-key", h, true, "ans-9", &at);
        let out = verb_actions_exec_extend_decide_with(
            &t,
            &json!({
                "handle": h, "approve": true,
                "answer_id": "ans-9", "answered_at": at, "sig": sig,
            }),
            &resolve,
        );
        assert_eq!(out["ok"], false, "{out}");
        // The request is still pending: the refusal decided nothing.
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "pending", "{stream}");
    }

    #[test]
    fn signed_decide_refuses_stale_and_future_envelopes() {
        let t = backend();
        let key = b"test-apply-key-s1";
        let resolve = || Ok::<Vec<u8>, String>(key.to_vec());
        // Stale: correctly signed but an hour old — a replay.
        let h = "fx-test-signed-old";
        pending_manual(&t, h);
        let old = iso(now_unix() - 3_600);
        let sig = test_sign(key, h, true, "ans-old", &old);
        let out = verb_actions_exec_extend_decide_with(
            &t,
            &json!({
                "handle": h, "approve": true,
                "answer_id": "ans-old", "answered_at": old, "sig": sig,
            }),
            &resolve,
        );
        assert_eq!(out["ok"], false, "{out}");
        // Far-future: correctly signed but beyond the skew allowance.
        let h2 = "fx-test-signed-new";
        pending_manual(&t, h2);
        let future = iso(now_unix() + 3_600);
        let sig2 = test_sign(key, h2, true, "ans-new", &future);
        let out = verb_actions_exec_extend_decide_with(
            &t,
            &json!({
                "handle": h2, "approve": true,
                "answer_id": "ans-new", "answered_at": future, "sig": sig2,
            }),
            &resolve,
        );
        assert_eq!(out["ok"], false, "{out}");
        for h in [h, h2] {
            let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
            assert_eq!(stream["session"]["ext_state"], "pending", "{stream}");
        }
    }

    #[test]
    fn unsigned_seat_decide_still_works() {
        // The signed path changes nothing for seat calls: no `sig`
        // field, same verbs, same outcomes, now labelled via seat.
        let t = backend();
        let h = "fx-test-seat-still";
        pending_manual(&t, h);
        let out = verb_actions_exec_extend_decide(&t, &json!({"handle": h, "approve": true}));
        assert_eq!(out["decided"], true, "{out}");
        assert_eq!(out["approved"], true, "{out}");
        assert_eq!(out["via"], "seat", "{out}");
        let stream = verb_actions_exec_stream(&t, &json!({"handle": h}));
        assert_eq!(stream["session"]["ext_state"], "approved", "{stream}");
    }
}
