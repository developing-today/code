//! needs-drew throat emission — PM-stack notifications Phase 1
//! (developing-today/code#8568).
//!
//! When a queue task blocks with reason-class `escalated-to-user`
//! and carries a needs-drew payload, the block path raises one
//! alert into the PM aggregator's alert throat so Drew's phone
//! hears about it. The throat dedupes by (source, key), so the
//! raise is idempotent: the CLI half and the broker half may both
//! post for the same block, and a retry is safe.
//!
//! Wire contract (fixed; the aggregator lane owns the receiving
//! side — do not deviate):
//!
//!   POST http://127.0.0.1:4870/api/alerts
//!     (env override: FLEET_ALERTS_URL, the full URL)
//!   Authorization: Bearer <token>
//!     (token = the Bitwarden secret named by
//!     [`ALERTS_INTAKE_TOKEN_SECRET`], resolved by name through
//!     fleet-secrets — never read from a file here)
//!   {"source": "fleet-queue",
//!    "key": "needs-drew:<ledgerId>",
//!    "severity": "warning",
//!    "title": <question>,
//!    "body": <context or blocker text>,
//!    "needsDrew": {"ledgerId": ..., "question": ...,
//!                  "context": ...,            // when present
//!                  "options": [{"id": ..., "label": ...}],  // non-empty
//!                  "links": [...]}}          // when non-empty
//!
//! The transport is a minimal HTTP/1.1 POST over a plain
//! `TcpStream`, in the style of client.rs's direct-broker leg:
//! the throat is a loopback plain-http endpoint, so no TLS or
//! HTTP-client dependency is needed for this one call.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

/// The alerts-intake token's name in Bitwarden Secrets Manager,
/// resolved by name through fleet-secrets at post time. The value
/// itself is never written to a file or a log line.
pub const ALERTS_INTAKE_TOKEN_SECRET: &str = "pm-aggregator.alerts-intake-token";

/// The default throat endpoint; `FLEET_ALERTS_URL` overrides it
/// (the full URL, path included).
pub const DEFAULT_ALERTS_URL: &str = "http://127.0.0.1:4870/api/alerts";

/// One answer option offered to Drew inside the needs-drew push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsUserOption {
    pub id: String,
    pub label: String,
}

/// The block-side needs-drew payload: everything the throat body
/// is built from. `ledger_id` is the PM ledger item the question
/// belongs to (an opaque id string, as the ledger names it);
/// `blocker` is the /block call's blocker text, used as the alert
/// body when no richer `context` was supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockNeedsUser {
    pub ledger_id: String,
    pub question: String,
    pub context: Option<String>,
    pub options: Vec<NeedsUserOption>,
    pub links: Vec<String>,
    pub blocker: String,
}

/// Build the exact throat POST body for one block. Pure — the
/// unit tests pin the shape (the aggregator lane parses it).
pub fn build_throat_body(block: &BlockNeedsUser) -> Value {
    let mut needs_drew = json!({
        "ledgerId": block.ledger_id,
        "question": block.question,
    });
    if let Some(context) = &block.context {
        needs_drew["context"] = json!(context);
    }
    if !block.options.is_empty() {
        needs_drew["options"] = Value::Array(
            block
                .options
                .iter()
                .map(|o| json!({"id": o.id, "label": o.label}))
                .collect(),
        );
    }
    if !block.links.is_empty() {
        needs_drew["links"] = json!(block.links);
    }
    // The alert body is the context when the caller supplied one,
    // else the block's blocker text (the contract's wording).
    let body = block.context.as_deref().unwrap_or(&block.blocker);
    json!({
        "source": "fleet-queue",
        "key": format!("needs-drew:{}", block.ledger_id),
        "severity": "warning",
        "title": block.question,
        "body": body,
        "needsDrew": needs_drew,
    })
}

/// The throat URL for this process: `FLEET_ALERTS_URL` when set
/// and non-empty, else the loopback default.
pub fn alerts_url() -> String {
    alerts_url_from_env(std::env::var("FLEET_ALERTS_URL").ok().as_deref())
}

fn alerts_url_from_env(value: Option<&str>) -> String {
    match value {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => DEFAULT_ALERTS_URL.to_string(),
    }
}

/// Split an `http://host:port[/path]` URL into (host:port, path).
/// Plain http only — the throat is a loopback endpoint.
fn parse_http_url(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("alerts URL must be http://host:port[/path], got {url:?}"))?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    if hostport.is_empty() {
        return Err(format!("alerts URL has no host: {url:?}"));
    }
    Ok((hostport.to_string(), path))
}

/// One minimal HTTP/1.1 JSON POST (client.rs transport style):
/// Content-Length, Connection: close, status line parsed from the
/// reply. Returns the parsed JSON body on a 2xx; any transport
/// failure or non-2xx status is an Err naming the status only —
/// never the token, never the payload.
fn post_json(url: &str, token: &str, body: &Value) -> Result<Value, String> {
    let (hostport, path) = parse_http_url(url)?;
    let payload = serde_json::to_vec(body).map_err(|e| format!("alerts body encode: {e}"))?;
    let mut s = TcpStream::connect(&hostport)
        .map_err(|e| format!("cannot connect to alerts throat at {hostport}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    s.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("alerts throat write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("alerts throat read failed: {e}"))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("alerts throat bad reply")?;
    let head_txt = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u64 = head_txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("alerts throat HTTP {status}"));
    }
    let text = String::from_utf8_lossy(&raw[split + 4..]).to_string();
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("alerts throat returned non-JSON: {e}"))
}

/// Raise one needs-drew alert into the throat for `nd`, with the
/// intake `token` resolved by the caller (by name, through
/// fleet-secrets). Returns the response's `answerUrl` when the
/// throat includes one (top-level, or nested under `alert`);
/// an empty string when the raise succeeded without one. The
/// raise is idempotent at the throat ((source, key) dedupe), so
/// posting twice for one block is safe.
pub fn post_needs_user(token: &str, nd: &BlockNeedsUser) -> Result<String, String> {
    post_needs_user_to(&alerts_url(), token, nd)
}

/// [`post_needs_user`] against an explicit URL (tests, overrides).
pub fn post_needs_user_to(url: &str, token: &str, nd: &BlockNeedsUser) -> Result<String, String> {
    let resp = post_json(url, token, &build_throat_body(nd))?;
    let answer = resp
        .get("answerUrl")
        .or_else(|| resp.get("alert").and_then(|a| a.get("answerUrl")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(answer)
}

/// Post an already-built plain (non-needs-drew) alert body to the
/// throat at `url` (#9177: the closed-bound detective's alert
/// carries no needsDrew object — the aggregator owns the receiving
/// side for both shapes). Reuses [`post_json`], so the transport
/// and error contract match the needs-drew raise exactly; the
/// caller resolves `token` the same way (by name, through
/// fleet-secrets) and the throat dedupes by (source, key) either
/// way. Ok(()) on any 2xx; Err names the status only.
pub fn post_plain_alert_to(url: &str, token: &str, body: &Value) -> Result<(), String> {
    post_json(url, token, body).map(|_| ())
}

/// The full detail set for a fleet actions exec extension push
/// (#13774, tiered policy — design amendment on the issue). The
/// broker verb assembles it; queue_broker maps the verb response
/// into this shape and posts the built body. Every tier's push
/// carries the same detail; only the framing differs
/// (notification vs deny-window vs manual ask).
pub struct ExtensionAlert {
    /// Alert key, ordinal-scoped by the verb:
    /// `actions-exec-ext:<handle>:<n>`.
    pub key: String,
    pub handle: String,
    /// Seat-supplied session context (task/issue link) recorded
    /// at open; None when the seat named none.
    pub context: Option<String>,
    pub host_repo: String,
    pub workflow: String,
    pub seat: String,
    pub run_id: Option<i64>,
    pub run_attempt: Option<i64>,
    pub created_at: String,
    /// Granted lifespan at open (max_until - created_at), seconds.
    pub granted_s: i64,
    /// Session age at request time, seconds.
    pub elapsed_s: i64,
    pub lease_until: String,
    pub max_until: String,
    pub extend_s: i64,
    pub tier: i64,
    /// Extensions granted before this request, and their total
    /// seconds; this request's own grant is the +1 / +extend_s.
    pub ext_count_before: i64,
    pub ext_total_s_before: i64,
    /// Tier 2 only: when silence approves.
    pub decide_after: Option<String>,
    /// The session's hard deadline under the pending request.
    pub grace_until: String,
    pub reason: String,
}

fn minutes_ceil(s: i64) -> i64 {
    (s + 59) / 60
}

/// Build the alerts-intake body for a fleet actions exec
/// extension request (#13774). Tier 1 is a plain notification
/// (no needsDrew block — the #9177 shape); tier 2's needsDrew
/// offers deny / approve-now and states that silence approves
/// at the window end; tier 3 is the v1 manual approve/deny ask.
/// The PM stack renders the needsDrew block as Drew's approve/
/// deny choice; the answer comes back as a seat-run
/// `/actions-exec-extend-decide` (the phone -> broker write-back
/// is the PM stack's next phase). Deduped by the throat on
/// (source, key); the key carries the extension ordinal, so
/// successive extensions on one session do not suppress each
/// other's pushes.
pub fn build_extension_alert_body(info: &ExtensionAlert) -> Value {
    let minutes = minutes_ceil(info.extend_s);
    let after_count = info.ext_count_before + 1;
    let after_total_min = minutes_ceil(info.ext_total_s_before + info.extend_s);
    let before_total_min = minutes_ceil(info.ext_total_s_before);
    let mut detail = format!(
        "Session {handle} (seat {seat}) on {repo}, workflow {workflow} run {run} attempt {attempt}. \
         Opened {created}; granted lifespan {granted} min, elapsed {elapsed} min; lease_until {lease}, max_until {max}. \
         Ask: +{minutes} min. Reason given: {reason} \
         Cumulative extensions: {bc} totalling {bt} min before this one; {ac} totalling {at} min counting it.",
        handle = info.handle,
        seat = info.seat,
        repo = info.host_repo,
        workflow = info.workflow,
        run = info
            .run_id
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".to_string()),
        attempt = info
            .run_attempt
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".to_string()),
        created = info.created_at,
        granted = minutes_ceil(info.granted_s),
        elapsed = minutes_ceil(info.elapsed_s),
        lease = info.lease_until,
        max = info.max_until,
        reason = if info.reason.is_empty() {
            "-"
        } else {
            info.reason.as_str()
        },
        bc = info.ext_count_before,
        bt = before_total_min,
        ac = after_count,
        at = after_total_min,
    );
    if let Some(ctx) = &info.context {
        detail.push_str(&format!(" Session context: {ctx}"));
    }
    let mut links: Vec<String> = Vec::new();
    if let Some(id) = info.run_id {
        let mut u = format!("https://github.com/{}/actions/runs/{id}", info.host_repo);
        if let Some(a) = info.run_attempt {
            u.push_str(&format!("/attempts/{a}"));
        }
        links.push(u);
    }
    if let Some(ctx) = &info.context {
        if ctx.starts_with("http") {
            links.push(ctx.clone());
        }
    }
    links.push("https://github.com/developing-today/code/issues/13774".to_string());
    match info.tier {
        1 => json!({
            "source": "fleet-actions-exec",
            "key": info.key,
            "severity": "info",
            "title": format!(
                "Actions exec {}: +{} min extension auto-approved (tier 1)",
                info.handle, minutes
            ),
            "body": format!(
                "{detail} Tier 1: the first extension within the auto caps is granted automatically; no action needed."
            ),
        }),
        2 => {
            let decide_after = info.decide_after.as_deref().unwrap_or("");
            let question = format!(
                "Actions exec session {} (+{} min, tier 2) approves itself at {} unless you deny — let it?",
                info.handle, minutes, decide_after
            );
            let context = format!(
                "{detail} Tier 2 deny window: silence approves automatically at {decide_after}; \
                 deny ends the session at its current cap (hard deadline {grace}); approve-now applies it immediately. \
                 Answers land via a seat running: fleet actions exec decide {handle} --deny|--approve.",
                grace = info.grace_until,
                handle = info.handle,
            );
            json!({
                "source": "fleet-actions-exec",
                "key": info.key,
                "severity": "warning",
                "title": question,
                "body": context,
                "needsDrew": {
                    "ledgerId": info.key,
                    "question": question,
                    "context": context,
                    "options": [
                        {"id": "deny", "label": "Deny — end at current cap"},
                        {"id": "approve", "label": format!("Approve now (+{minutes} min)")},
                    ],
                    "links": links,
                },
            })
        }
        _ => {
            let question = format!(
                "Extend actions exec session {} past its max lifespan by {} min (tier 3 — manual approval)?",
                info.handle, minutes
            );
            let context = format!(
                "{detail} Tier 3: manual approval only. Approving adds {extend_s}s to the max and the lease; \
                 denying — or no answer before the decision grace ends at {grace} — lets the session expire.",
                extend_s = info.extend_s,
                grace = info.grace_until,
            );
            json!({
                "source": "fleet-actions-exec",
                "key": info.key,
                "severity": "warning",
                "title": question,
                "body": context,
                "needsDrew": {
                    "ledgerId": info.key,
                    "question": question,
                    "context": context,
                    "options": [
                        {"id": "approve", "label": format!("Approve +{minutes} min")},
                        {"id": "deny", "label": "Deny — let it expire"},
                    ],
                    "links": links,
                },
            })
        }
    }
}

/// Parse the snake_case `needs_drew` object the /block body
/// carries (the shape cmd_block sends and the broker receives):
/// {ledger_id, question, context?, options?, links?}, with
/// `blocker` taken from the block itself. Returns None when the
/// object is absent or lacks a usable ledger_id / question — a
/// malformed payload must never fail or fake a block.
pub fn from_block_request(req: &Value, blocker: &str) -> Option<BlockNeedsUser> {
    let nd = req.get("needs_drew").filter(|v| v.is_object())?;
    let ledger_id = match nd.get("ledger_id") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return None,
    };
    let question = nd
        .get("question")
        .and_then(|q| q.as_str())
        .filter(|q| !q.trim().is_empty())?
        .to_string();
    let context = nd
        .get("context")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string());
    let options = nd
        .get("options")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    Some(NeedsUserOption {
                        id: o.get("id")?.as_str()?.to_string(),
                        label: o.get("label")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let links = nd
        .get("links")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Some(BlockNeedsUser {
        ledger_id,
        question,
        context,
        options,
        links,
        blocker: blocker.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn full_block() -> BlockNeedsUser {
        BlockNeedsUser {
            ledger_id: "ledger-42".to_string(),
            question: "Ship the red or the blue build?".to_string(),
            context: Some("Both pass CI; blue is 2 days older.".to_string()),
            options: vec![
                NeedsUserOption {
                    id: "red".to_string(),
                    label: "Ship red".to_string(),
                },
                NeedsUserOption {
                    id: "blue".to_string(),
                    label: "Ship blue".to_string(),
                },
            ],
            links: vec!["https://example.invalid/pr/1".to_string()],
            blocker: "escalated: awaiting Drew's pick".to_string(),
        }
    }

    #[test]
    fn throat_body_matches_the_fixed_contract_shape() {
        let body = build_throat_body(&full_block());
        assert_eq!(
            body,
            json!({
                "source": "fleet-queue",
                "key": "needs-drew:ledger-42",
                "severity": "warning",
                "title": "Ship the red or the blue build?",
                "body": "Both pass CI; blue is 2 days older.",
                "needsDrew": {
                    "ledgerId": "ledger-42",
                    "question": "Ship the red or the blue build?",
                    "context": "Both pass CI; blue is 2 days older.",
                    "options": [
                        {"id": "red", "label": "Ship red"},
                        {"id": "blue", "label": "Ship blue"},
                    ],
                    "links": ["https://example.invalid/pr/1"],
                },
            })
        );
    }

    #[test]
    fn throat_body_omits_absent_optionals_and_falls_back_to_blocker() {
        let block = BlockNeedsUser {
            ledger_id: "99".to_string(),
            question: "Proceed?".to_string(),
            context: None,
            options: vec![],
            links: vec![],
            blocker: "blocked on Drew".to_string(),
        };
        let body = build_throat_body(&block);
        assert_eq!(body["key"], json!("needs-drew:99"));
        assert_eq!(body["title"], json!("Proceed?"));
        // No context: the blocker text is the alert body.
        assert_eq!(body["body"], json!("blocked on Drew"));
        let nd = &body["needsDrew"];
        assert_eq!(nd["ledgerId"], json!("99"));
        assert_eq!(nd["question"], json!("Proceed?"));
        assert!(nd.get("context").is_none());
        assert!(nd.get("options").is_none());
        assert!(nd.get("links").is_none());
    }

    #[test]
    fn alerts_url_env_override_wins_and_blank_falls_back() {
        assert_eq!(alerts_url_from_env(None), DEFAULT_ALERTS_URL);
        assert_eq!(alerts_url_from_env(Some("  ")), DEFAULT_ALERTS_URL);
        assert_eq!(
            alerts_url_from_env(Some("http://127.0.0.1:9999/x")),
            "http://127.0.0.1:9999/x"
        );
    }

    #[test]
    fn parse_http_url_splits_hostport_and_path() {
        assert_eq!(
            parse_http_url(DEFAULT_ALERTS_URL).unwrap(),
            ("127.0.0.1:4870".to_string(), "/api/alerts".to_string())
        );
        assert_eq!(
            parse_http_url("http://127.0.0.1:4870").unwrap(),
            ("127.0.0.1:4870".to_string(), "/".to_string())
        );
        assert!(parse_http_url("https://127.0.0.1:4870/api/alerts").is_err());
        assert!(parse_http_url("http:///api/alerts").is_err());
    }

    #[test]
    fn from_block_request_round_trips_the_snake_case_shape() {
        let req = json!({
            "blocker": "waiting on Drew",
            "needs_drew": {
                "ledger_id": "ledger-7",
                "question": "Which scope?",
                "context": "ctx",
                "options": [{"id": "a", "label": "A"}],
                "links": ["https://example.invalid/x"],
            },
        });
        let nd = from_block_request(&req, "waiting on Drew").unwrap();
        assert_eq!(nd.ledger_id, "ledger-7");
        assert_eq!(nd.question, "Which scope?");
        assert_eq!(nd.context.as_deref(), Some("ctx"));
        assert_eq!(nd.options.len(), 1);
        assert_eq!(nd.links.len(), 1);
        assert_eq!(nd.blocker, "waiting on Drew");
        // And the throat body built from it carries the same key.
        assert_eq!(build_throat_body(&nd)["key"], json!("needs-drew:ledger-7"));
    }

    #[test]
    fn from_block_request_accepts_a_numeric_ledger_id() {
        let req = json!({"needs_drew": {"ledger_id": 123, "question": "Q?"}});
        let nd = from_block_request(&req, "b").unwrap();
        assert_eq!(nd.ledger_id, "123");
    }

    fn sample_alert(tier: i64) -> ExtensionAlert {
        ExtensionAlert {
            key: "actions-exec-ext:fx-sample:2".to_string(),
            handle: "fx-sample".to_string(),
            context: Some("https://github.com/developing-today/code/issues/13774".to_string()),
            host_repo: "dezren39/fleet".to_string(),
            workflow: "fleet-exec.yml".to_string(),
            seat: "archer".to_string(),
            run_id: Some(123_456_789_012),
            run_attempt: Some(2),
            created_at: "2026-11-23T22:00:00Z".to_string(),
            granted_s: 1800,
            elapsed_s: 1500,
            lease_until: "2026-11-23T22:30:00Z".to_string(),
            max_until: "2026-11-23T22:30:00Z".to_string(),
            extend_s: 1800,
            tier,
            ext_count_before: 1,
            ext_total_s_before: 1800,
            decide_after: Some("2026-11-23T22:40:00Z".to_string()),
            grace_until: "2026-11-23T22:45:00Z".to_string(),
            reason: "install step still running".to_string(),
        }
    }

    #[test]
    fn extension_alert_tier1_is_a_plain_notification() {
        // Oracle: tier 1 notifies only — no needsDrew choice
        // block — and still carries the full detail set.
        let body = build_extension_alert_body(&sample_alert(1));
        assert_eq!(body["source"], json!("fleet-actions-exec"));
        assert_eq!(body["key"], json!("actions-exec-ext:fx-sample:2"));
        assert_eq!(body["severity"], json!("info"));
        assert!(body.get("needsDrew").is_none(), "{body}");
        let text = body["body"].as_str().unwrap();
        assert!(text.contains("fx-sample"), "{text}");
        assert!(text.contains("archer"), "{text}");
        assert!(text.contains("install step still running"), "{text}");
        assert!(text.contains("auto caps"), "{text}");
    }

    #[test]
    fn extension_alert_tier2_offers_deny_first_and_states_silence_approves() {
        // Oracle: tier 2's push must say it approves itself at
        // the window end, offer deny as the active choice, and
        // deep-link the exact run attempt.
        let body = build_extension_alert_body(&sample_alert(2));
        let nd = &body["needsDrew"];
        assert_eq!(nd["options"][0]["id"], json!("deny"), "{body}");
        assert_eq!(nd["options"][1]["id"], json!("approve"), "{body}");
        let q = nd["question"].as_str().unwrap();
        assert!(q.contains("approves itself at 2026-11-23T22:40:00Z"), "{q}");
        let links = nd["links"].as_array().unwrap();
        assert_eq!(
            links[0],
            json!("https://github.com/dezren39/fleet/actions/runs/123456789012/attempts/2")
        );
        assert_eq!(nd["ledgerId"], json!("actions-exec-ext:fx-sample:2"));
        let ctx = nd["context"].as_str().unwrap();
        assert!(ctx.contains("current cap"), "{ctx}");
    }

    #[test]
    fn extension_alert_tier3_is_the_manual_ask() {
        // Oracle: tier 3 keeps the v1 approve/deny shape, with
        // the decision-grace consequence stated.
        let body = build_extension_alert_body(&sample_alert(3));
        let nd = &body["needsDrew"];
        assert_eq!(nd["options"][0]["id"], json!("approve"), "{body}");
        assert_eq!(nd["options"][1]["id"], json!("deny"), "{body}");
        let ctx = nd["context"].as_str().unwrap();
        assert!(ctx.contains("manual approval only"), "{ctx}");
        assert!(ctx.contains("2026-11-23T22:45:00Z"), "{ctx}");
    }

    #[test]
    fn from_block_request_rejects_absent_or_incomplete_payloads() {
        assert!(from_block_request(&json!({}), "b").is_none());
        assert!(from_block_request(&json!({"needs_drew": {"question": "Q?"}}), "b").is_none());
        assert!(from_block_request(&json!({"needs_drew": {"ledger_id": "x"}}), "b").is_none());
        assert!(
            from_block_request(
                &json!({"needs_drew": {"ledger_id": " ", "question": "Q?"}}),
                "b"
            )
            .is_none()
        );
    }
}
